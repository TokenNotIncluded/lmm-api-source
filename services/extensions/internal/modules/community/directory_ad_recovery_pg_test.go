// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package community

import (
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"strings"
	"testing"
	"time"

	pb "github.com/TokenNotIncluded/lmm-api/extensions/internal/corepb"
	_ "github.com/lib/pq"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

// TestLostReceiptPublishesAndRefundsOnce needs COMMUNITY_TEST_DSN pointing at a
// fresh database with schema.sql already installed. Pay and Refund both lose
// their responses; the original receipts are read back and written once.
func TestLostReceiptPublishesAndRefundsOnce(t *testing.T) {
	dsn := os.Getenv("COMMUNITY_TEST_DSN")
	if dsn == "" {
		t.Skip("COMMUNITY_TEST_DSN not set")
	}
	db, err := sql.Open("postgres", dsn)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = db.Close() })
	ctx := context.Background()
	if err = db.PingContext(ctx); err != nil {
		t.Fatal(err)
	}
	in := directoryAdInput{Name: "Example", URL: "https://example.com/a", Summary: "short", Description: "longer", BidCents: 100, RequestID: "adrequest00000001"}
	in, err = normalizeDirectoryAdInput(in)
	if err != nil {
		t.Fatal(err)
	}
	digest := directoryAdContentHash(in)
	quote := strings.Repeat("a", 64)
	paid := time.Now().Unix()
	_, err = db.ExecContext(ctx, `DELETE FROM lmm_community.directory_ads WHERE request_id=$1`, in.RequestID)
	if err != nil {
		t.Fatal(err)
	}
	_, err = db.ExecContext(ctx, `DELETE FROM lmm_community.directory_ad_intents WHERE request_id=$1`, in.RequestID)
	if err != nil {
		t.Fatal(err)
	}
	_, err = db.ExecContext(ctx, `INSERT INTO lmm_community.directory_ad_intents(request_id,owner_user_id,name,url,summary,description,bid_cents,content_sha256,quote_id,quota,quote_expires_at,payment_state) VALUES($1,42,$2,$3,$4,$5,100,$6,$7,500,$8,'prepared')`, in.RequestID, in.Name, in.URL, in.Summary, in.Description, digest, quote, paid+600)
	if err != nil {
		t.Fatal(err)
	}
	core := &recoveryAdCore{
		payErr: status.Error(codes.Unavailable, "lost"),
		queryPayReceipt: &pb.DirectoryAdReceipt{
			RequestId: in.RequestID, CommandSha256: strings.Repeat("b", 64), Outcome: "succeeded",
			PaymentId: quote, ChargedQuota: 500, PaidAt: paid, AdExpiresAt: paid + 30*24*60*60,
		},
	}
	svc, err := New(ctx, db, "https://example.test", Options{AdCore: core})
	if err != nil {
		t.Fatal(err)
	}
	session := pb.CredentialKind_CREDENTIAL_KIND_SESSION
	mux := http.NewServeMux()
	svc.Register(mux, fakeAuth{
		"user":  {UserId: 42, PlatformLevel: 1, CredentialKind: session},
		"admin": {UserId: 7, PlatformLevel: 5, CredentialKind: session},
	})
	body := fmt.Sprintf(`{"name":%q,"url":%q,"summary":%q,"description":%q,"bid_cents":100,"expected_quota":500,"request_id":%q,"quote_id":%q,"content_sha256":%q}`, in.Name, in.URL, in.Summary, in.Description, in.RequestID, quote, digest)
	call := func(method, path, token, payload string) (int, map[string]any) {
		req := httptest.NewRequest(method, path, strings.NewReader(payload))
		req.Header.Set("Authorization", "Bearer "+token)
		rec := httptest.NewRecorder()
		mux.ServeHTTP(rec, req)
		var out map[string]any
		_ = json.Unmarshal(rec.Body.Bytes(), &out)
		return rec.Code, out
	}
	code, out := call("POST", "/api/ai-directory/ads", "user", body)
	if code != 200 || out["success"] != true {
		t.Fatalf("publish: %d %s", code, out)
	}
	code, out = call("POST", "/api/ai-directory/ads", "user", body)
	if code != 200 {
		t.Fatalf("replay: %d %v", code, out)
	}
	var ads int
	var state, payment string
	if err = db.QueryRowContext(ctx, `SELECT count(*) FROM lmm_community.directory_ads WHERE request_id=$1`, in.RequestID).Scan(&ads); err != nil || ads != 1 {
		t.Fatalf("ads=%d err=%v", ads, err)
	}
	if err = db.QueryRowContext(ctx, `SELECT payment_state,payment_id FROM lmm_community.directory_ad_intents WHERE request_id=$1`, in.RequestID).Scan(&state, &payment); err != nil || state != "paid" || payment != quote {
		t.Fatalf("intent %s %s %v", state, payment, err)
	}
	if core.payCalls != 1 {
		t.Fatalf("pay calls=%d", core.payCalls)
	}
	sum := sha256.Sum256([]byte(in.RequestID))
	core.refundErr = status.Error(codes.Unavailable, "lost")
	core.queryRefundReceipt = &pb.DirectoryAdReceipt{
		RequestId: fmt.Sprintf("ad-refund-%x", sum), PaymentId: quote, CommandSha256: strings.Repeat("c", 64),
		Outcome: "succeeded", RefundedQuota: 500, RefundedAt: time.Now().Unix(), RefundId: "journal-1",
	}
	var id int64
	if err = db.QueryRowContext(ctx, `SELECT id FROM lmm_community.directory_ads WHERE request_id=$1`, in.RequestID).Scan(&id); err != nil {
		t.Fatal(err)
	}
	code, out = call("POST", fmt.Sprintf("/api/ai-directory/ads/%d/hide", id), "admin", "")
	if code != 200 || out["success"] != true {
		t.Fatalf("hide: %d %v", code, out)
	}
	var refunded string
	if err = db.QueryRowContext(ctx, `SELECT status,refund_status FROM lmm_community.directory_ads WHERE id=$1`, id).Scan(&state, &refunded); err != nil || state != "hidden" || refunded != "refunded" {
		t.Fatalf("placement %s %s %v", state, refunded, err)
	}
	if core.refundCalls != 1 {
		t.Fatalf("refund calls=%d", core.refundCalls)
	}
}
