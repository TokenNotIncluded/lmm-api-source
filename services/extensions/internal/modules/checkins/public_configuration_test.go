package checkins

import "testing"

func TestAssistantPublicConfigurationUsesPublishedRewardRange(t *testing.T) {
	revision := int64(7)
	policy := Policy{Enabled: true, MinQuota: "-2", MaxQuota: "-1", CalendarOffsetSeconds: Offset, CreditsPerUSD: 500000, SourceRevision: SourceRevision}
	got, err := publicConfiguration(&policy, &revision)
	if err != nil || got != (PublishedConfiguration{Revision: 7, Enabled: true}) {
		t.Fatal("published legacy negative bounds must use the actual zero-clamped reward range", got, err)
	}
	policy.MinQuota, policy.MaxQuota = "11", "5"
	got, err = publicConfiguration(&policy, &revision)
	if err != nil || got.MinCredits != 11 || got.MaxCredits != 11 {
		t.Fatal("public range differs from reward authority", got, err)
	}
	policy.MinQuota = "01"
	if _, err = publicConfiguration(&policy, &revision); err == nil {
		t.Fatal("malformed stored policy became an available reward")
	}
	if _, err = publicConfiguration(nil, &revision); err == nil {
		t.Fatal("revision without a policy accepted")
	}
	if got, err = publicConfiguration(nil, nil); err != nil || got != (PublishedConfiguration{}) {
		t.Fatal("unpublished module enabled rewards", got, err)
	}
}
