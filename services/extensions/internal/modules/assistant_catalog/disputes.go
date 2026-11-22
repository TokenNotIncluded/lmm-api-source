package assistant_catalog

import (
	"context"
	"encoding/json"
	"strconv"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
)

// Deliberately flat: neither the original Command nor nested evidence records
// can cross the model boundary. Frozen and live money remain separate facts.
type bountyDispute struct {
	ID                         string     `json:"id"`
	ProjectID                  string     `json:"project_id"`
	ChallengeID                string     `json:"challenge_id"`
	OwnerUserID                identifier `json:"owner_user_id"`
	ParticipantUserID          identifier `json:"participant_user_id"`
	OpenedByUserID             identifier `json:"opened_by_user_id"`
	AgainstUserID              identifier `json:"against_user_id"`
	ResolvedByUserID           identifier `json:"resolved_by_user_id"`
	Status                     string     `json:"status"`
	Reason                     string     `json:"reason"`
	Statement                  string     `json:"statement"`
	Resolution                 string     `json:"resolution"`
	ProjectTitleSnapshot       string     `json:"project_title_snapshot"`
	RepositoryURLSnapshot      string     `json:"repository_url_snapshot"`
	ProjectRulesSnapshot       string     `json:"project_rules_snapshot"`
	ProjectEscrowQuotaSnapshot int64      `json:"project_escrow_quota_snapshot"`
	ChallengeStatusSnapshot    string     `json:"challenge_status_snapshot"`
	IssueURLSnapshot           string     `json:"issue_url_snapshot"`
	PullRequestURLSnapshot     string     `json:"pull_request_url_snapshot"`
	SubmissionNoteSnapshot     string     `json:"submission_note_snapshot"`
	ReviewNoteSnapshot         string     `json:"review_note_snapshot"`
	RewardQuotaSnapshot        int64      `json:"reward_quota_snapshot"`
	TipQuotaSnapshot           int64      `json:"tip_quota_snapshot"`
	ProjectTitle               string     `json:"project_title"`
	RepositoryURL              string     `json:"repository_url"`
	ProjectRules               string     `json:"project_rules"`
	CurrentProjectEscrowQuota  int64      `json:"current_project_escrow_quota"`
	ChallengeStatus            string     `json:"challenge_status"`
	IssueURL                   string     `json:"issue_url"`
	PullRequestURL             string     `json:"pull_request_url"`
	SubmissionNote             string     `json:"submission_note"`
	ReviewNote                 string     `json:"review_note"`
	RewardQuota                int64      `json:"reward_quota"`
	TipQuota                   int64      `json:"tip_quota"`
	LiveEvidenceChanged        bool       `json:"live_evidence_changed"`
	CreatedAt                  int64      `json:"created_at"`
	UpdatedAt                  int64      `json:"updated_at"`
	ResolvedAt                 int64      `json:"resolved_at"`
	ResolutionDeadlineAt       *int64     `json:"resolution_deadline_at"`
}

func (d *bountyDispute) UnmarshalJSON(raw []byte) error {
	if err := requireBountyAmounts(raw, "project_escrow_quota_snapshot", "reward_quota_snapshot", "tip_quota_snapshot", "current_project_escrow_quota", "reward_quota", "tip_quota"); err != nil {
		return err
	}
	type wire bountyDispute
	var value wire
	if json.Unmarshal(raw, &value) != nil {
		return access.ErrUnavailable
	}
	*d = bountyDispute(value)
	return nil
}

func disputeProjection(d *bountyDispute) error {
	if !access.ID(d.ID) || !access.ID(d.ProjectID) || !access.ID(d.ChallengeID) ||
		d.OwnerUserID == "" || d.ParticipantUserID == "" ||
		d.OpenedByUserID != d.OwnerUserID && d.OpenedByUserID != d.ParticipantUserID ||
		d.CreatedAt < 0 || d.UpdatedAt < 0 || d.ResolvedAt < 0 || d.ResolutionDeadlineAt != nil {
		return access.ErrUnavailable
	}
	against := d.OwnerUserID
	if d.OpenedByUserID == d.OwnerUserID {
		against = d.ParticipantUserID
	}
	if d.AgainstUserID != against {
		return access.ErrUnavailable
	}
	switch d.Status {
	case "open", "resolved_paid", "resolved_denied":
	default:
		return access.ErrUnavailable
	}
	for _, amount := range []int64{d.ProjectEscrowQuotaSnapshot, d.RewardQuotaSnapshot, d.TipQuotaSnapshot, d.CurrentProjectEscrowQuota, d.RewardQuota, d.TipQuota} {
		if amount < 0 {
			return access.ErrUnavailable
		}
	}
	// These are evidence text, not executable arguments. Do not silently rewrite
	// it or expose an unbounded record when the existing domain is malformed.
	for _, value := range []string{d.Reason, d.Statement, d.Resolution, d.ProjectRulesSnapshot, d.SubmissionNoteSnapshot, d.ReviewNoteSnapshot, d.ProjectRules, d.SubmissionNote, d.ReviewNote} {
		if len(value) > 5000 {
			return access.ErrUnavailable
		}
	}
	return nil
}

func (t *tool) bountyDisputes(ctx context.Context, in assistant.AgentInvocation, p access.Principal, a bountyArguments) (any, error) {
	limit, status := 50, ""
	if a.Limit != nil {
		limit = *a.Limit
	}
	if a.Status != nil {
		status = *a.Status
	}
	if limit < 1 || limit > 100 {
		return nil, access.ErrInvalid
	}
	switch status {
	case "", "open", "resolved_paid", "resolved_denied":
	default:
		return nil, access.ErrInvalid
	}
	administrative := a.View == "admin_disputes"
	rows, err := t.executor.bounties.DisputesForViewer(ctx, in.Credential, administrative, status, limit)
	if err != nil {
		return nil, err
	}
	if len(rows) > limit {
		return nil, access.ErrUnavailable
	}
	out := bountyResult{OK: true, View: a.View, ReadOnly: true, WriteActionsRequireUIConfirmation: true,
		MCPEquivalent: "open_source_bounties.list_disputes", Status: status, Limit: limit,
		CreditsPerUSD: 500000, UnsupportedViews: []string{}, Disputes: []bountyDispute{}, HasMore: len(rows) == limit}
	actor := identifier(strconv.FormatInt(p.UserID, 10))
	bytes := 0
	for _, row := range rows {
		var d bountyDispute
		if domainProjection(row, &d) != nil || disputeProjection(&d) != nil || status != "" && d.Status != status {
			return nil, access.ErrUnavailable
		}
		if !administrative && d.OwnerUserID != actor && d.ParticipantUserID != actor {
			return nil, access.ErrForbidden
		}
		encoded, err := json.Marshal(d)
		if err != nil {
			return nil, access.ErrUnavailable
		}
		bytes += len(encoded)
		if bytes > 48<<10 {
			out.Truncated, out.HasMore = true, true
			continue
		}
		out.Disputes = append(out.Disputes, d)
	}
	return out, nil
}
