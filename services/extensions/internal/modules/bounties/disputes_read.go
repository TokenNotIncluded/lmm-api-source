package bounties

import (
	"context"
	"encoding/json"
)

// DisputesForViewer is a fixed read operation. The caller selects only the
// personal or administrative view, never an actor, command or permission bit.
// Personal reads remain party-scoped even for administrators. Core performs
// its normal Session and administrator policy checks before and after the read.
func (s *Service) DisputesForViewer(ctx context.Context, session string, administrative bool, status string, limit int) ([]map[string]any, error) {
	if s == nil || s.Store == nil || s.Funds == nil || session == "" {
		return nil, ErrForbidden
	}
	if limit < 1 || limit > 100 {
		return nil, ErrInvalid
	}
	switch status {
	case "", "open", "resolved_paid", "resolved_denied":
	default:
		return nil, ErrInvalid
	}
	actor, err := s.Funds.Authorize(ctx, session)
	if err != nil {
		return nil, err
	}
	if !validID(actor.UserID) || administrative && !actor.Admin {
		return nil, ErrForbidden
	}
	items := []map[string]any{}
	err = s.Store.Within(ctx, func(tx Tx) error {
		v, ok := tx.(ViewTx)
		if !ok {
			return ErrUnavailable
		}
		q := Query{Status: status, Limit: limit}
		if !administrative {
			q.PartyUserID = actor.UserID
		}
		rows, err := tx.List("disputes", q)
		if err != nil {
			return err
		}
		if len(rows) > limit {
			return ErrUnavailable
		}
		for _, raw := range rows {
			var d Dispute
			var snapshots struct {
				Project   json.RawMessage `json:"project_snapshot"`
				Challenge json.RawMessage `json:"challenge_snapshot"`
			}
			if json.Unmarshal(raw, &d) != nil || json.Unmarshal(raw, &snapshots) != nil ||
				disputeReadAmounts(raw, "tip_quota_snapshot") != nil ||
				disputeReadAmounts(snapshots.Project, "escrow_credit") != nil ||
				disputeReadAmounts(snapshots.Challenge, "reward_credit") != nil {
				return ErrUnavailable
			}
			if !validID(d.ID) || !validID(d.ProjectID) || !validID(d.ChallengeID) ||
				!validID(d.OwnerUserID) || !validID(d.ParticipantUserID) ||
				d.OpenedByUserID != d.OwnerUserID && d.OpenedByUserID != d.ParticipantUserID ||
				status != "" && d.Status != status ||
				d.Status != "open" && d.Status != "resolved_paid" && d.Status != "resolved_denied" {
				return ErrUnavailable
			}
			if !administrative && d.OwnerUserID != actor.UserID && d.ParticipantUserID != actor.UserID {
				return ErrForbidden
			}
			var projectRaw, challengeRaw json.RawMessage
			if err = tx.Get("projects", d.ProjectID, &projectRaw); err != nil {
				return err
			}
			if err = tx.Get("challenges", d.ChallengeID, &challengeRaw); err != nil {
				return err
			}
			var p Project
			var ch Challenge
			if json.Unmarshal(projectRaw, &p) != nil || json.Unmarshal(challengeRaw, &ch) != nil ||
				disputeReadAmounts(projectRaw, "escrow_credit") != nil ||
				disputeReadAmounts(challengeRaw, "reward_credit") != nil {
				return ErrUnavailable
			}
			if p.ID != d.ProjectID || ch.ID != d.ChallengeID || ch.ProjectID != p.ID ||
				p.OwnerUserID != d.OwnerUserID || ch.OwnerUserID != d.OwnerUserID || ch.ParticipantUserID != d.ParticipantUserID ||
				d.ProjectSnapshot.ID != p.ID || d.ChallengeSnapshot.ID != ch.ID || d.ChallengeSnapshot.ProjectID != p.ID {
				return ErrConflict
			}
			tips, err := v.Sum("notifications", Query{ChallengeID: ch.ID, TipsOnly: true})
			if err != nil {
				return err
			}
			if tips < 0 {
				return ErrUnavailable
			}
			ch.TipCredit = tips
			item := disputeView(d, p, ch)
			// Preserve the established flat display fields, but do not expose
			// stored commands or nested records containing confirmation material.
			delete(item, "evidence")
			delete(item, "project_snapshot")
			delete(item, "challenge_snapshot")
			// The domain has no resolution deadline for an already-open dispute.
			// Its seven-day appeal window is a different, opening-time rule.
			item["resolution_deadline_at"] = nil
			items = append(items, item)
		}
		return nil
	})
	if err != nil {
		return nil, err
	}
	current, err := s.Funds.Authorize(ctx, session)
	if err != nil {
		return nil, err
	}
	if current.UserID != actor.UserID || administrative && !current.Admin {
		return nil, ErrForbidden
	}
	return items, nil
}

// Validate the exact source amounts consumed by disputeView before decoding
// into integer structs, where a missing or null member would otherwise be zero.
func disputeReadAmounts(raw json.RawMessage, names ...string) error {
	var fields map[string]json.RawMessage
	if json.Unmarshal(raw, &fields) != nil || fields == nil {
		return ErrUnavailable
	}
	for _, name := range names {
		data, present := fields[name]
		var amount *int64
		if !present || json.Unmarshal(data, &amount) != nil || amount == nil || *amount < 0 {
			return ErrUnavailable
		}
	}
	return nil
}
