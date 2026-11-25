package assistant_catalog

import (
	"context"
	"encoding/json"
	"math/big"
	"strconv"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/assistant/access"
	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/bounties"
)

type bountyArguments struct {
	View      string     `json:"view"`
	ProjectID identifier `json:"project_id"`
	Page      *int       `json:"page"`
	PageSize  *int       `json:"page_size"`
	Status    *string    `json:"status"`
	Limit     *int       `json:"limit"`
}
type bountyProject struct {
	ID                   string           `json:"id"`
	OwnerUserID          identifier       `json:"owner_user_id"`
	Title                string           `json:"title"`
	Description          string           `json:"description"`
	DescriptionTruncated bool             `json:"description_truncated"`
	Rules                string           `json:"rules"`
	RulesTruncated       bool             `json:"rules_truncated"`
	RepositoryURL        string           `json:"repository_url"`
	Status               string           `json:"status"`
	RewardCredit         int64            `json:"reward_credit"`
	RewardQuota          int64            `json:"reward_quota"`
	RewardSlots          int              `json:"reward_slots"`
	EscrowCredit         int64            `json:"escrow_credit"`
	NetRewardCredit      int64            `json:"net_reward_credit"`
	PlatformFeeRateBPS   *int64           `json:"platform_fee_rate_bps,omitempty"`
	ParticipantCount     *int64           `json:"participant_count,omitempty"`
	ActiveChallengeCount *int64           `json:"active_challenge_count,omitempty"`
	OpenDisputeCount     *int64           `json:"open_dispute_count,omitempty"`
	ArchivedAt           int64            `json:"archived_at"`
	CreatedAt            int64            `json:"created_at"`
	UpdatedAt            int64            `json:"updated_at"`
	ViewerChallenge      *bountyChallenge `json:"viewer_challenge,omitempty"`
}
type bountyChallenge struct {
	ID                string     `json:"id"`
	ProjectID         string     `json:"project_id"`
	OwnerUserID       identifier `json:"owner_user_id"`
	ParticipantUserID identifier `json:"participant_user_id"`
	ProjectTitle      string     `json:"project_title,omitempty"`
	RepositoryURL     string     `json:"repository_url,omitempty"`
	Status            string     `json:"status"`
	RewardCredit      int64      `json:"reward_credit"`
	RewardQuota       int64      `json:"reward_quota"`
	TipCredit         int64      `json:"tip_credit"`
	GithubHandle      string     `json:"github_handle"`
	IssueURL          string     `json:"issue_url"`
	PullRequestURL    string     `json:"pull_request_url"`
	SubmissionNote    string     `json:"submission_note"`
	ReviewNote        string     `json:"review_note"`
	CreatedAt         int64      `json:"created_at"`
	UpdatedAt         int64      `json:"updated_at"`
}
type bountyLedger struct {
	ID                 string     `json:"id"`
	ProjectID          string     `json:"project_id"`
	ChallengeID        string     `json:"challenge_id"`
	UserID             identifier `json:"user_id"`
	CounterpartyUserID identifier `json:"counterparty_user_id"`
	Kind               string     `json:"kind"`
	Quota              int64      `json:"quota"`
	Note               string     `json:"note"`
	JournalIDs         []int64    `json:"journal_ids"`
	CreatedAt          int64      `json:"created_at"`
}
type bountyBoard struct {
	Items    []bountyProject `json:"items"`
	Total    int64           `json:"total"`
	Page     int             `json:"page"`
	PageSize int             `json:"page_size"`
}
type bountyDetail struct {
	Project    bountyProject     `json:"project"`
	Challenges []bountyChallenge `json:"challenges"`
	Ledger     []bountyLedger    `json:"ledger"`
}
type bountyResult struct {
	OK                                bool              `json:"ok"`
	View                              string            `json:"view"`
	ReadOnly                          bool              `json:"read_only"`
	MCPEquivalent                     string            `json:"mcp_equivalent"`
	WriteActionsRequireUIConfirmation bool              `json:"write_actions_require_ui_confirmation"`
	Board                             *bountyBoard      `json:"board,omitempty"`
	Detail                            *bountyDetail     `json:"detail,omitempty"`
	Projects                          []bountyProject   `json:"projects,omitempty"`
	Challenges                        []bountyChallenge `json:"challenges,omitempty"`
	Disputes                          []bountyDispute   `json:"disputes,omitempty"`
	Status                            string            `json:"status,omitempty"`
	Limit                             int               `json:"limit,omitempty"`
	Page                              int               `json:"page,omitempty"`
	PageSize                          int               `json:"page_size,omitempty"`
	HasMore                           bool              `json:"has_more"`
	Truncated                         bool              `json:"truncated"`
	CreditsPerUSD                     int64             `json:"credits_per_usd"`
	UnsupportedViews                  []string          `json:"unsupported_views"`
}
type bountyGuide struct {
	OK                     bool               `json:"ok"`
	ReadOnly               bool               `json:"read_only"`
	Enabled                bool               `json:"enabled"`
	Code                   string             `json:"code,omitempty"`
	EligibleDeliverables   []string           `json:"eligible_deliverables"`
	Steps                  []string           `json:"steps"`
	PlatformFeePercent     json.Number        `json:"platform_fee_percent"`
	PlatformFeeBasisPoints int64              `json:"platform_fee_basis_points"`
	Limits                 bounties.FeeLimits `json:"limits"`
	Page                   string             `json:"page"`
	Message                string             `json:"message"`
}

// Every amount below is present even when zero in the existing typed domain
// records. Refuse an incomplete/null read instead of inventing a zero amount.
func requireBountyAmounts(raw []byte, names ...string) error {
	var fields map[string]json.RawMessage
	if json.Unmarshal(raw, &fields) != nil || fields == nil {
		return access.ErrUnavailable
	}
	for _, name := range names {
		data, found := fields[name]
		var amount *int64
		if !found || json.Unmarshal(data, &amount) != nil || amount == nil {
			return access.ErrUnavailable
		}
	}
	return nil
}
func (p *bountyProject) UnmarshalJSON(raw []byte) error {
	if err := requireBountyAmounts(raw, "reward_credit", "escrow_credit", "net_reward_credit"); err != nil {
		return err
	}
	type wire bountyProject
	var value wire
	if json.Unmarshal(raw, &value) != nil {
		return access.ErrUnavailable
	}
	*p = bountyProject(value)
	return nil
}
func (c *bountyChallenge) UnmarshalJSON(raw []byte) error {
	if err := requireBountyAmounts(raw, "reward_credit", "tip_credit"); err != nil {
		return err
	}
	type wire bountyChallenge
	var value wire
	if json.Unmarshal(raw, &value) != nil {
		return access.ErrUnavailable
	}
	*c = bountyChallenge(value)
	return nil
}
func (l *bountyLedger) UnmarshalJSON(raw []byte) error {
	if err := requireBountyAmounts(raw, "quota"); err != nil {
		return err
	}
	type wire bountyLedger
	var value wire
	if json.Unmarshal(raw, &value) != nil {
		return access.ErrUnavailable
	}
	*l = bountyLedger(value)
	return nil
}

func domainProjection(value any, out any) error {
	raw, err := json.Marshal(value)
	if err != nil || len(raw) > 8<<20 || json.Unmarshal(raw, out) != nil {
		return access.ErrUnavailable
	}
	return nil
}
func projectProjection(p *bountyProject, full bool) error {
	if !access.ID(p.ID) || p.Title == "" || p.OwnerUserID == "" || p.RewardCredit < 0 || p.EscrowCredit < 0 || p.NetRewardCredit < 0 {
		return access.ErrUnavailable
	}
	desc, rules := 400, 400
	if full {
		desc, rules = 2000, 5000
	}
	p.Description, p.DescriptionTruncated = clipped(p.Description, desc)
	p.Rules, p.RulesTruncated = clipped(p.Rules, rules)
	p.RewardQuota = p.RewardCredit
	if p.ViewerChallenge != nil {
		return challengeProjection(p.ViewerChallenge)
	}
	return nil
}
func challengeProjection(c *bountyChallenge) error {
	if !access.ID(c.ID) || !access.ID(c.ProjectID) || c.ParticipantUserID == "" || c.RewardCredit < 0 || c.TipCredit < 0 {
		return access.ErrUnavailable
	}
	c.RewardQuota = c.RewardCredit
	// Domain validation already bounds these human-written fields. Reject a bad
	// projection instead of silently editing evidence or returning private blobs.
	if len(c.SubmissionNote) > 5000 || len(c.ReviewNote) > 5000 {
		return access.ErrUnavailable
	}
	return nil
}
func (t *tool) bounty(ctx context.Context, in assistant.AgentInvocation, p access.Principal) (any, error) {
	if access.Missing(t.executor.bounties) {
		return nil, access.ErrUnavailable
	}
	if t.definition.Name == "get_bounty_guide" {
		var a struct{}
		if access.Decode(in.Call.Arguments, &a) != nil {
			return nil, access.ErrInvalid
		}
		fee, err := t.executor.bounties.Config(ctx)
		if err != nil {
			return nil, err
		}
		ratio, ok := new(big.Rat).SetString(string(fee.RatePercent))
		if !ok || fee.RateBasisPoints < 0 || fee.RateBasisPoints > 10000 || ratio.Cmp(new(big.Rat).SetFrac64(fee.RateBasisPoints, 100)) != 0 {
			return nil, access.ErrUnavailable
		}
		return bountyGuide{OK: true, ReadOnly: true, Enabled: false, Code: "retired", PlatformFeePercent: fee.RatePercent, PlatformFeeBasisPoints: fee.RateBasisPoints, Limits: fee.Limits, Page: "/support",
			Steps:   []string{"Use the original account to review existing obligations or open a settlement dispute."},
			Message: "The bounty program is closed to new work. Existing escrow, receipts and disputes remain available to their parties."}, nil
	}
	var a bountyArguments
	if access.Decode(in.Call.Arguments, &a) != nil {
		return nil, access.ErrInvalid
	}
	if a.View == "disputes" || a.View == "admin_disputes" {
		if a.ProjectID != "" || a.Page != nil || a.PageSize != nil {
			return nil, access.ErrInvalid
		}
		return t.bountyDisputes(ctx, in, p, a)
	}
	if a.Status != nil || a.Limit != nil {
		return nil, access.ErrInvalid
	}
	page, size := 1, 20
	if a.View == "accepted" || a.View == "owned" {
		size = 50
	}
	if a.Page != nil {
		page = *a.Page
	}
	if a.PageSize != nil {
		size = *a.PageSize
	}
	if page < 1 || page > 1000000 || size < 1 || size > 50 {
		return nil, access.ErrInvalid
	}
	if a.View == "detail" {
		if a.ProjectID == "" || a.ProjectID == "0" || a.Page != nil || a.PageSize != nil {
			return nil, access.ErrInvalid
		}
	} else if a.ProjectID != "" {
		return nil, access.ErrInvalid
	}
	out := bountyResult{OK: true, View: a.View, ReadOnly: true, WriteActionsRequireUIConfirmation: true, CreditsPerUSD: 500000, UnsupportedViews: []string{}}
	actor := identifier(strconv.FormatInt(p.UserID, 10))
	switch a.View {
	case "board", "detail":
		return nil, bounties.ErrRetired
	case "accepted", "owned":
		action := "list_accepted"
		if a.View == "owned" {
			action = "list_owned"
		}
		// Only these two literal read commands can reach the existing dispatcher.
		// It rechecks the current personal Session and resource authority; no Actor ID, admin bit, operation,
		// confirmation, amount or mutation action is taken from the tool input.
		rows, err := t.executor.bounties.BuiltinInvoke(ctx, bounties.Invocation{Actor: bounties.Actor{Token: in.Credential}, Command: bounties.Command{Action: action}, Page: page, PageSize: size})
		if err != nil {
			return nil, err
		}
		if len(rows.Items) > size || rows.Receipt != nil || rows.Project != nil || rows.Challenge != nil || rows.Dispute != nil {
			return nil, access.ErrUnavailable
		}
		for _, raw := range rows.Items {
			if a.View == "owned" {
				var v bountyProject
				if json.Unmarshal(raw, &v) != nil || v.OwnerUserID != actor {
					return nil, access.ErrForbidden
				}
				if err = projectProjection(&v, false); err != nil {
					return nil, err
				}
				out.Projects = append(out.Projects, v)
			} else {
				var v bountyChallenge
				if json.Unmarshal(raw, &v) != nil || v.ParticipantUserID != actor {
					return nil, access.ErrForbidden
				}
				if err = challengeProjection(&v); err != nil {
					return nil, err
				}
				out.Challenges = append(out.Challenges, v)
			}
		}
		out.Page = page
		out.PageSize = size
		out.HasMore = len(rows.Items) == size
		out.MCPEquivalent = "open_source_bounties." + action
	default:
		return nil, access.ErrInvalid
	}
	return out, nil
}
