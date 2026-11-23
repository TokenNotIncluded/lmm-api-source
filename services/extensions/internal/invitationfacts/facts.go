// Package invitationfacts is the closed, read-only Core-to-host invitation contract.
package invitationfacts

import (
	"bytes"
	"encoding/json"
	"io"
	"strings"
	"time"
)

const Feature = "assistant.invitation_facts.v1"
const MaxSafeInteger int64 = 9007199254740991

type Balances struct {
	WalletSignedCredits            int64  `json:"wallet_signed_credits"`
	AffiliateAvailableCredits      int64  `json:"affiliate_available_credits"`
	AffiliateDebtCredits           int64  `json:"affiliate_debt_credits"`
	OpeningAffiliateHistoryCredits int64  `json:"opening_affiliate_history_credits"`
	HistoryScope                   string `json:"history_scope"`
}
type Facts struct {
	Version                       int       `json:"version"`
	OK                            bool      `json:"ok"`
	Scope                         string    `json:"scope"`
	ActorUserID                   int64     `json:"actor_user_id"`
	AuthVersion                   int64     `json:"auth_version"`
	CheckedAtUnix                 int64     `json:"checked_at_unix"`
	InvitationCode                *string   `json:"invitation_code"`
	ActorEmail                    *string   `json:"actor_email"`
	NativeRegisteredReferrals     int64     `json:"native_registered_referrals"`
	LegacyInvitedCountAvailable   bool      `json:"legacy_invited_count_available"`
	LegacyInvitedCount            *int64    `json:"legacy_invited_count"`
	AutomaticSignupRewardsEnabled bool      `json:"automatic_signup_rewards_enabled"`
	AutomaticSignupRewardsReason  string    `json:"automatic_signup_rewards_reason"`
	BalanceStatus                 string    `json:"balance_status"`
	Balances                      *Balances `json:"balances"`
	CreditUnit                    string    `json:"credit_unit"`
	FinancialAuthority            bool      `json:"financial_authority"`
}

func object(raw []byte, keys []string, nullable map[string]bool) (map[string]json.RawMessage, error) {
	d := json.NewDecoder(bytes.NewReader(raw))
	t, e := d.Token()
	if e != nil || t != json.Delim('{') {
		return nil, io.ErrUnexpectedEOF
	}
	fields := map[string]json.RawMessage{}
	for d.More() {
		t, e = d.Token()
		if e != nil {
			return nil, e
		}
		key, ok := t.(string)
		if !ok {
			return nil, io.ErrUnexpectedEOF
		}
		if _, ok = fields[key]; ok {
			return nil, io.ErrUnexpectedEOF
		}
		var value json.RawMessage
		if d.Decode(&value) != nil {
			return nil, io.ErrUnexpectedEOF
		}
		fields[key] = value
	}
	t, e = d.Token()
	if e != nil || t != json.Delim('}') {
		return nil, io.ErrUnexpectedEOF
	}
	var extra any
	if d.Decode(&extra) != io.EOF {
		return nil, io.ErrUnexpectedEOF
	}
	if len(fields) != len(keys) {
		return nil, io.ErrUnexpectedEOF
	}
	for _, k := range keys {
		v, ok := fields[k]
		if !ok || (!nullable[k] && bytes.Equal(bytes.TrimSpace(v), []byte("null"))) {
			return nil, io.ErrUnexpectedEOF
		}
	}
	return fields, nil
}
func (b *Balances) UnmarshalJSON(raw []byte) error {
	_, err := object(raw, []string{"wallet_signed_credits", "affiliate_available_credits", "affiliate_debt_credits", "opening_affiliate_history_credits", "history_scope"}, nil)
	if err != nil {
		return err
	}
	type plain Balances
	var out plain
	if json.Unmarshal(raw, &out) != nil {
		return io.ErrUnexpectedEOF
	}
	*b = Balances(out)
	return nil
}
func (f *Facts) UnmarshalJSON(raw []byte) error {
	if len(raw) == 0 || len(raw) > 16384 {
		return io.ErrUnexpectedEOF
	}
	_, err := object(raw, []string{"version", "ok", "scope", "actor_user_id", "auth_version", "checked_at_unix", "invitation_code", "actor_email", "native_registered_referrals", "legacy_invited_count_available", "legacy_invited_count", "automatic_signup_rewards_enabled", "automatic_signup_rewards_reason", "balance_status", "balances", "credit_unit", "financial_authority"}, map[string]bool{"invitation_code": true, "actor_email": true, "legacy_invited_count": true, "balances": true})
	if err != nil {
		return err
	}
	type plain Facts
	var out plain
	if json.Unmarshal(raw, &out) != nil {
		return io.ErrUnexpectedEOF
	}
	*f = Facts(out)
	return nil
}
func ValidCode(s string) bool {
	if len(s) != 4 && len(s) != 32 {
		return false
	}
	for _, c := range s {
		if len(s) == 4 {
			if !((c >= '0' && c <= '9') || (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z')) {
				return false
			}
		} else if !((c >= '0' && c <= '9') || (c >= 'a' && c <= 'f')) {
			return false
		}
	}
	return true
}
func safe(n int64) bool { return n >= -MaxSafeInteger && n <= MaxSafeInteger }
func (f Facts) Valid(user int64) bool {
	if f.Version != 1 || !f.OK || f.Scope != "self" || user <= 0 || f.ActorUserID != user || f.AuthVersion <= 0 || f.CheckedAtUnix < time.Now().Unix()-30 || f.CheckedAtUnix > time.Now().Unix()+5 || f.NativeRegisteredReferrals < 0 || !safe(f.NativeRegisteredReferrals) || f.LegacyInvitedCountAvailable || f.LegacyInvitedCount != nil || f.AutomaticSignupRewardsEnabled || f.AutomaticSignupRewardsReason != "registration_records_attribution_without_issuing_reward" || f.CreditUnit != "core_credit" || f.FinancialAuthority {
		return false
	}
	if f.InvitationCode != nil && !ValidCode(*f.InvitationCode) {
		return false
	}
	if f.ActorEmail != nil && (len(*f.ActorEmail) == 0 || len(*f.ActorEmail) > 254 || strings.ContainsAny(*f.ActorEmail, "\x00\r\n")) {
		return false
	}
	if f.Balances == nil {
		return f.BalanceStatus == "financial_projection_unavailable"
	}
	b := f.Balances
	return f.BalanceStatus == "available" && safe(b.WalletSignedCredits) && safe(b.OpeningAffiliateHistoryCredits) && b.AffiliateAvailableCredits >= 0 && safe(b.AffiliateAvailableCredits) && b.AffiliateDebtCredits >= 0 && safe(b.AffiliateDebtCredits) && (b.AffiliateAvailableCredits == 0 || b.AffiliateDebtCredits == 0) && b.HistoryScope == "validated_opening_or_native_zero_basis_only"
}
