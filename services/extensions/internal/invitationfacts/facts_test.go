package invitationfacts

import (
	"encoding/json"
	"strings"
	"testing"
	"time"
)

func testFacts() Facts {
	code, email := "aB09", "owner@example.invalid"
	return Facts{Version: 1, OK: true, Scope: "self", ActorUserID: 101, AuthVersion: 2, CheckedAtUnix: time.Now().Unix(), InvitationCode: &code, ActorEmail: &email, NativeRegisteredReferrals: 0, AutomaticSignupRewardsReason: "registration_records_attribution_without_issuing_reward", BalanceStatus: "available", Balances: &Balances{WalletSignedCredits: -3, AffiliateDebtCredits: 2, OpeningAffiliateHistoryCredits: 7, HistoryScope: "validated_opening_or_native_zero_basis_only"}, CreditUnit: "core_credit"}
}
func TestInvitationFactsRequiredAmountsAndAuthority(t *testing.T) {
	good := testFacts()
	raw, _ := json.Marshal(good)
	var parsed Facts
	if json.Unmarshal(raw, &parsed) != nil || !parsed.Valid(101) {
		t.Fatal("valid signed facts rejected")
	}
	for _, change := range []func(map[string]any){
		func(v map[string]any) { delete(v, "actor_user_id") },
		func(v map[string]any) { v["actor_user_id"] = nil },
		func(v map[string]any) { v["actor_user_id"] = 202.0 },
		func(v map[string]any) { v["automatic_signup_rewards_enabled"] = true },
		func(v map[string]any) { v["financial_authority"] = true },
		func(v map[string]any) { v["checked_at_unix"] = float64(time.Now().Unix() - 60) },
		func(v map[string]any) { v["legacy_invited_count"] = 0.0 },
		func(v map[string]any) { delete(v["balances"].(map[string]any), "affiliate_debt_credits") },
		func(v map[string]any) { v["balances"].(map[string]any)["affiliate_debt_credits"] = nil },
		func(v map[string]any) { v["balances"].(map[string]any)["affiliate_debt_credits"] = 1.5 },
		func(v map[string]any) { v["balances"].(map[string]any)["affiliate_available_credits"] = 3.0 },
		func(v map[string]any) {
			v["balances"].(map[string]any)["wallet_signed_credits"] = float64(MaxSafeInteger + 1)
		},
	} {
		var v map[string]any
		_ = json.Unmarshal(raw, &v)
		change(v)
		bad, _ := json.Marshal(v)
		var got Facts
		if json.Unmarshal(bad, &got) == nil && got.Valid(101) {
			t.Fatal("incomplete or unauthorized facts accepted", string(bad))
		}
	}
	duplicate := strings.Replace(string(raw), "\"ok\":true", "\"ok\":true,\"ok\":true", 1)
	if json.Unmarshal([]byte(duplicate), &parsed) == nil {
		t.Fatal("duplicate key accepted")
	}
	good.Balances = nil
	good.BalanceStatus = "financial_projection_unavailable"
	good.InvitationCode = nil
	if !good.Valid(101) {
		t.Fatal("explicit unavailable balance or missing source code rejected")
	}
	good.BalanceStatus = "available"
	if good.Valid(101) {
		t.Fatal("missing balance converted to available zero")
	}
}
func TestInvitationCodePreservesLegacyCaseWithoutInventingCodes(t *testing.T) {
	for _, s := range []string{"aB09", "Ab09", strings.Repeat("a", 32)} {
		if !ValidCode(s) {
			t.Fatal("valid code refused")
		}
	}
	for _, s := range []string{"", "abc", "ab_1", strings.Repeat("A", 32), strings.Repeat("a", 31), "四字代码"} {
		if ValidCode(s) {
			t.Fatal("invalid code accepted")
		}
	}
}
