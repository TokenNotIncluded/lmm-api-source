package payments

import (
	"bytes"
	"context"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

type dualLegacyCore struct {
	*legacyFake
	selected []int64
}

func (c *dualLegacyCore) LookupLegacyProof(ctx context.Context, v LegacyCallback) (LegacyProof, error) {
	c.selected = append(c.selected, v.ChannelID)
	return c.legacyFake.LookupLegacyProof(ctx, v)
}
func TestPancakeDualCurrencyAliasPostgresPending(t *testing.T) {
	db := openPancakeRecoveryPG(t)
	if _, err := db.Exec(`DROP TABLE IF EXISTS payment_pancake_wallet_inbox,payment_pancake_subscription_inbox,payment_pancake_subscription_checkouts; DROP FUNCTION IF EXISTS guard_pancake_subscription_recovery();` + SubscriptionPancakeSchema); err != nil {
		t.Fatal(err)
	}
	client := &http.Client{Transport: transportFunc(func(*http.Request) (*http.Response, error) {
		t.Fatal("callback performed provider I/O")
		return nil, ErrInvalid
	})}
	usd, cny := dualPancakeAdapters(t, client)
	core := &dualLegacyCore{legacyFake: &legacyFake{lookupErr: ErrNotFound, acceptErr: ErrNotFound}}
	wallet, err := New(newMemory(), core, usd, cny)
	if err != nil {
		t.Fatal(err)
	}
	recovery := &PancakeSubscriptions{db: db, adapters: map[int64]*Pancake{5001: usd, 5003: cny}}
	recovery.AttachWallet(wallet)
	mux := http.NewServeMux()
	mux.Handle("POST /api/waffo-pancake/webhook/{environment}", recovery.LegacyWebhookHandler(wallet))
	for _, tc := range []struct {
		currency string
		channel  int64
	}{{"USD", 5001}, {"CNY", 5003}, {"", 0}} {
		t.Run(fmt.Sprintf("currency_%s_channel_%d", tc.currency, tc.channel), func(t *testing.T) {
			raw := fmt.Sprintf(`{"eventId":"evt_dual_%d","eventType":"order.completed","mode":"prod","storeId":"STO_0123456789abcdefghijkl","data":{"orderMerchantExternalId":"legacy_dual_%d","orderId":"ORD_dual_%d","currency":%q,"amount":"13.27"}}`, tc.channel, tc.channel, tc.channel, tc.currency)
			for duplicate := 0; duplicate < 2; duplicate++ {
				request := httptest.NewRequest("POST", "/api/waffo-pancake/webhook/prod", strings.NewReader(raw))
				request.Header.Set("X-Waffo-Signature", " original-dual-signature ")
				response := httptest.NewRecorder()
				mux.ServeHTTP(response, request)
				if response.Code != 404 {
					t.Fatal("unknown original falsely acknowledged", response.Code, response.Body.String())
				}
			}
			var payload, signature []byte
			var state string
			var count, attempts int
			var resultNull bool
			if err := db.QueryRow(`SELECT count(*) FROM payment_pancake_wallet_inbox WHERE channel_id=$1`, tc.channel).Scan(&count); err != nil || count != 1 {
				t.Fatal("lost/doubled raw evidence", count, err)
			}
			if err := db.QueryRow(`SELECT payload,signature,state,attempts,result IS NULL FROM payment_pancake_wallet_inbox WHERE channel_id=$1`, tc.channel).Scan(&payload, &signature, &state, &attempts, &resultNull); err != nil || !bytes.Equal(payload, []byte(raw)) || string(signature) != " original-dual-signature " || state != "pending" || attempts != 2 || !resultNull {
				t.Fatal("pending original changed", state, attempts, err)
			}
		})
	}
	if fmt.Sprint(core.selected) != "[5001 5001 5003 5003]" || core.credits != 0 || core.postings != 0 {
		t.Fatal("unverified currency granted credit or selected wrong channel", core.selected, core.credits)
	}
	// Reopening the recovery component keeps channel zero unbound and all raw rows.
	restart := &PancakeSubscriptions{db: db, adapters: recovery.adapters}
	restart.AttachWallet(wallet)
	if _, err = db.Exec(`UPDATE payment_pancake_wallet_inbox SET next_attempt=clock_timestamp()`); err != nil {
		t.Fatal(err)
	}
	if err = restart.recoverWallet(context.Background(), 10); err == nil {
		t.Fatal("unknown basis was closed during recovery")
	}
	var pending int
	if err = db.QueryRow(`SELECT count(*) FROM payment_pancake_wallet_inbox WHERE state='pending' AND result IS NULL AND attempts=3`).Scan(&pending); err != nil || pending != 3 {
		t.Fatal("unresolved originals lost on restart", pending, err)
	}
}
