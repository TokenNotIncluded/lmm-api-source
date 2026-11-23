package assistant_catalog

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"testing"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/bounties"
)

func dispute() map[string]any {
	return map[string]any{
		"id": "d-1", "project_id": "p-1", "challenge_id": "c-1", "owner_user_id": "7", "participant_user_id": "8", "opened_by_user_id": 8, "against_user_id": 7,
		"resolved_by_user_id": "", "status": "open", "reason": "review disagreement", "statement": "Actual statement", "resolution": "",
		"project_title_snapshot": "Original project", "project_title": "Current project", "repository_url": "https://github.com/example/project", "project_rules": "Actual acceptance rule",
		"project_escrow_quota_snapshot": int64(999), "reward_quota_snapshot": int64(101), "tip_quota_snapshot": int64(3),
		"current_project_escrow_quota": int64(898), "reward_quota": int64(101), "tip_quota": int64(7),
		"created_at": int64(100), "updated_at": int64(102), "resolved_at": int64(0), "resolution_deadline_at": nil, "live_evidence_changed": true,
		"evidence":           map[string]any{"confirmation": "private-confirmation", "operation_id": "private-command"},
		"project_snapshot":   map[string]any{"hidden": "private-project-body"},
		"challenge_snapshot": map[string]any{"evidence": map[string]any{"confirmation": "private-nested-command"}},
		"credential":         "private-credential", "unrecognized_private_field": "private-unused",
	}
}

func TestCatalogueDisputesUseFixedViewsAndStripCommands(t *testing.T) {
	for _, view := range []string{"disputes", "admin_disputes"} {
		t.Run(view, func(t *testing.T) {
			e, auth, _, b := fixture(t)
			b.disputes = []map[string]any{dispute()}
			// Above the float64 exact-integer range: credits must stay exact.
			b.disputes[0]["reward_quota"] = int64(9007199254740993)
			target, in := invocation(t, e, "get_bounty_data", `{"view":"`+view+`","status":"open","limit":9}`)
			out, err := target.Execute(context.Background(), in)
			if err != nil || auth.calls != 2 || b.calls != 1 || b.token != catalogueSession || b.administrative != (view == "admin_disputes") || b.status != "open" || b.limit != 9 || b.invocation.Command.Action != "" {
				t.Fatal(err, auth.calls, b.calls, b.invocation)
			}
			for _, forbidden := range []string{"private-confirmation", "private-command", "private-project-body", "private-nested-command", "private-credential", "private-unused", `"evidence"`, `"project_snapshot"`, `"challenge_snapshot"`} {
				if strings.Contains(string(out.ResultJSON), forbidden) {
					t.Fatal("private command material leaked", forbidden)
				}
			}
			var value struct {
				View     string          `json:"view"`
				Disputes []bountyDispute `json:"disputes"`
			}
			if json.Unmarshal(out.ResultJSON, &value) != nil || value.View != view || len(value.Disputes) != 1 {
				t.Fatal(string(out.ResultJSON))
			}
			d := value.Disputes[0]
			if d.OwnerUserID != "7" || d.ParticipantUserID != "8" || d.OpenedByUserID != "8" || d.AgainstUserID != "7" || d.RewardQuota != 9007199254740993 || d.TipQuota != 7 || d.TipQuotaSnapshot != 3 || d.CurrentProjectEscrowQuota != 898 || d.ProjectEscrowQuotaSnapshot != 999 || d.ResolutionDeadlineAt != nil || !d.LiveEvidenceChanged || d.CreatedAt != 100 || d.UpdatedAt != 102 {
				t.Fatal("dispute facts changed", d)
			}
		})
	}
}

func TestCatalogueDisputeArgumentsCannotChooseActorPolicyOrMutation(t *testing.T) {
	for _, args := range []string{
		`{"view":"disputes","admin":true}`, `{"view":"admin_disputes","actor":7}`, `{"view":"disputes","party_user_id":8}`,
		`{"view":"admin_disputes","action":"resolve_dispute"}`, `{"view":"disputes","resolution":"approve"}`,
		`{"view":"disputes","page":1}`, `{"view":"disputes","page_size":1}`, `{"view":"disputes","project_id":1}`,
		`{"view":"disputes","limit":0}`, `{"view":"admin_disputes","limit":101}`, `{"view":"disputes","limit":1.5}`, `{"view":"disputes","limit":null}`,
		`{"view":"disputes","status":null}`, `{"view":"disputes","status":"paid"}`, `{"view":"disputes","status":" open "}`,
		`{"view":"disputes","status":"open","status":"resolved_paid"}`, `{"view":"board","status":"open"}`, `{"view":"owned","limit":1}`,
	} {
		t.Run(args, func(t *testing.T) {
			e, _, _, b := fixture(t)
			target, in := invocation(t, e, "get_bounty_data", args)
			if _, err := target.Execute(context.Background(), in); !errors.Is(err, access.ErrInvalid) || b.calls != 0 {
				t.Fatal("invalid request reached domain", err, b.calls)
			}
		})
	}
}

func TestCataloguePersonalDisputeScopeAndCoreAdminDenialCannotBeForged(t *testing.T) {
	e, _, _, b := fixture(t)
	d := dispute()
	d["owner_user_id"], d["opened_by_user_id"], d["against_user_id"] = "9", 8, 9
	b.disputes = []map[string]any{d}
	target, in := invocation(t, e, "get_bounty_data", `{"view":"disputes"}`)
	in.Actor.Level = 100 // Host/model metadata is never administrator authority.
	if _, err := target.Execute(context.Background(), in); !errors.Is(err, access.ErrForbidden) || b.administrative {
		t.Fatal("another party's dispute returned", err)
	}
	b.err = bounties.ErrForbidden
	target, in = invocation(t, e, "get_bounty_data", `{"view":"admin_disputes"}`)
	in.Actor.Level = 100
	if _, err := target.Execute(context.Background(), in); !errors.Is(err, access.ErrForbidden) || !b.administrative || b.token != catalogueSession {
		t.Fatal("host privilege replaced Core policy", err)
	}
}

func TestCatalogueDisputeMissingNullInvalidMoneyIsNeverZero(t *testing.T) {
	for _, field := range []string{"project_escrow_quota_snapshot", "reward_quota_snapshot", "tip_quota_snapshot", "current_project_escrow_quota", "reward_quota", "tip_quota"} {
		for _, mode := range []string{"missing", "null", "string", "fraction", "negative"} {
			t.Run(field+"/"+mode, func(t *testing.T) {
				e, _, _, b := fixture(t)
				d := dispute()
				switch mode {
				case "missing":
					delete(d, field)
				case "null":
					d[field] = nil
				case "string":
					d[field] = "0"
				case "fraction":
					d[field] = 0.5
				case "negative":
					d[field] = -1
				}
				b.disputes = []map[string]any{d}
				target, in := invocation(t, e, "get_bounty_data", `{"view":"disputes"}`)
				if _, err := target.Execute(context.Background(), in); !errors.Is(err, access.ErrUnavailable) {
					t.Fatal("invalid amount accepted", err)
				}
			})
		}
	}
	e, _, _, b := fixture(t)
	d := dispute()
	for _, field := range []string{"project_escrow_quota_snapshot", "reward_quota_snapshot", "tip_quota_snapshot", "current_project_escrow_quota", "reward_quota", "tip_quota"} {
		d[field] = int64(0)
	}
	b.disputes = []map[string]any{d}
	target, in := invocation(t, e, "get_bounty_data", `{"view":"disputes"}`)
	if _, err := target.Execute(context.Background(), in); err != nil {
		t.Fatal("actual zero rejected", err)
	}
}

func TestCatalogueDisputeResultsRemainBoundedWithoutRewritingEvidence(t *testing.T) {
	e, _, _, b := fixture(t)
	for i := 0; i < 50; i++ {
		d := dispute()
		d["statement"] = strings.Repeat("e", 3000)
		b.disputes = append(b.disputes, d)
	}
	target, in := invocation(t, e, "get_bounty_data", `{"view":"disputes"}`)
	out, err := target.Execute(context.Background(), in)
	var value struct {
		Disputes  []bountyDispute `json:"disputes"`
		Truncated bool            `json:"truncated"`
		HasMore   bool            `json:"has_more"`
	}
	if err != nil || json.Unmarshal(out.ResultJSON, &value) != nil || len(out.ResultJSON) > 60<<10 || !value.Truncated || !value.HasMore || len(value.Disputes) == 0 || len(value.Disputes) >= 50 {
		t.Fatal("read was unbounded or silently truncated", err, len(out.ResultJSON))
	}
	for _, d := range value.Disputes {
		if len(d.Statement) != 3000 {
			t.Fatal("evidence rewritten")
		}
	}
}
