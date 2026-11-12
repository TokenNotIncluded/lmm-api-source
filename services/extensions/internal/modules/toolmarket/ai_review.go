// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
package toolmarket

import (
	"context"
	"sort"
	"time"

	"github.com/TokenNotIncluded/lmm-api/extensions/internal/modules/marketreview"
)

func (m *Module) ConfigureAIReview(engine *marketreview.Engine) error {
	if m == nil || engine == nil || engine.Policy == nil || engine.Classifier == nil {
		return ErrInvalid
	}
	m.review = engine
	return nil
}

type preparedAIReview struct {
	before  string
	history marketreview.History
}

func (m *Module) prepareAIReview(ctx context.Context, user int64, id, version string, mode marketreview.Mode, route marketreview.Route, credential ...string) (*preparedAIReview, error) {
	var s WebServiceRecord
	var v webSnapshot
	if err := m.store.view(func(d database) error {
		w := webState(&d)
		var err error
		s, err = webOwned(w, user, id)
		if err != nil {
			return err
		}
		var ok bool
		v, ok = w.Versions[version]
		if !ok || v.Version.ServiceID != id {
			return ErrMissing
		}
		if s.DraftVersionID != version || v.Version.Status != "draft" && v.Version.Status != "rejected" {
			return ErrConflict
		}
		return nil
	}); err != nil {
		return nil, err
	}
	if err := m.invalidatePrepared(ctx, v.AIReview); err != nil {
		return nil, err
	}
	prepared := &preparedAIReview{before: digest([]any{s, v}), history: v.AIReview}
	parts := []string{v.Version.Name, v.Version.Description}
	for _, tool := range v.Tools {
		parts = append(parts, tool.Name, tool.Description)
	}
	text, inputErr := marketreview.PublicText(parts...)
	if err := prepared.history.Enqueue("tool", id, version, s.OwnerID, mode, route, text, inputErr, m.now().Unix()); err != nil {
		return nil, err
	}
	if v.Version.Visibility != "public" {
		prepared.history.Manual("private_listing", m.now().Unix())
	}
	if mode != marketreview.Auto || len(prepared.history.Jobs) == 0 || m.review == nil {
		return prepared, nil
	}
	job := &prepared.history.Jobs[len(prepared.history.Jobs)-1]
	job.ApprovalPriorVersion = s.LiveVersionID
	port, ok := m.review.Classifier.(marketreview.ToolAuthority)
	if !ok || !port.AutoApproveTools() || job.Status != "pending" || len(credential) != 1 {
		return prepared, nil
	}
	quotes, err := legacyVersionCreditQuotes(id, v)
	if err != nil {
		return prepared, nil
	}
	ids := make([]string, 0, len(quotes))
	for _, quote := range quotes {
		ids = append(ids, quote.ID)
	}
	sort.Strings(ids)
	job.AdmissionAttempted = true
	revision, err := port.AdmitTool(ctx, credential[0], job.Request(), ids, s.LiveVersionID)
	if err == nil {
		job.AdmissionRevision = revision
	}
	return prepared, nil
}

func (m *Module) invalidatePrepared(ctx context.Context, h marketreview.History) error {
	for _, job := range h.Jobs {
		if job.Token != h.Token || !job.AdmissionAttempted {
			continue
		}
		if m.review == nil {
			return ErrUnavailable
		}
		port, ok := m.review.Classifier.(marketreview.ToolAuthority)
		if !ok {
			return ErrUnavailable
		}
		if err := port.InvalidateTool(ctx, job.OwnerID, job.Token); err != nil {
			return ErrUnavailable
		}
	}
	return nil
}

func (m *Module) invalidateAIReview(ctx context.Context, user int64, id, version string, reviewer bool) error {
	var histories []marketreview.History
	if err := m.store.view(func(d database) error {
		w := webState(&d)
		s, ok := w.Services[id]
		if !ok || s.Status == "deleted" {
			return ErrMissing
		}
		if !reviewer && s.OwnerID != user {
			return ErrMissing
		}
		for vid, v := range w.Versions {
			if v.Version.ServiceID == id && (version == "" && (vid == s.DraftVersionID || vid == s.LiveVersionID) || version == vid) {
				histories = append(histories, v.AIReview)
			}
		}
		return nil
	}); err != nil {
		return err
	}
	for _, h := range histories {
		if err := m.invalidatePrepared(ctx, h); err != nil {
			return err
		}
	}
	return nil
}

func (m *Module) AIReviewHistory(ctx context.Context, credential, account, id, version string) ([]marketreview.Record, error) {
	user, err := m.WebAuthorize(ctx, credential, account, "toolmarket.read")
	if err != nil {
		return nil, err
	}
	var owner int64
	err = m.store.view(func(d database) error {
		if d.Web == nil {
			return ErrMissing
		}
		s, ok := d.Web.Services[id]
		if !ok || s.Status == "deleted" {
			return ErrMissing
		}
		owner = s.OwnerID
		return nil
	})
	if err != nil {
		return nil, err
	}
	if owner != user {
		if _, err = m.WebAuthorize(ctx, credential, account, "toolmarket.review"); err != nil {
			return nil, err
		}
	}
	rows := []marketreview.Record{}
	err = m.store.view(func(d database) error {
		w := webState(&d)
		s, ok := w.Services[id]
		if !ok || s.Status == "deleted" || s.OwnerID != owner {
			return ErrMissing
		}
		if version == "" {
			version = s.DraftVersionID
			if version == "" {
				version = s.LiveVersionID
			}
		}
		v, ok := w.Versions[version]
		if !ok || v.Version.ServiceID != id {
			return ErrMissing
		}
		rows = v.AIReview.Records()
		return nil
	})
	return rows, err
}

// Verify the current remote schema without borrowing a user's session. This is
// a non-executing metadata read, bounded by the existing MCP network policy.
func (m *Module) reviewTechnical(ctx context.Context, j marketreview.Job) ([]string, string, string) {
	var v webSnapshot
	var credential webCredential
	if err := m.store.view(func(d database) error {
		if d.Web == nil {
			return ErrMissing
		}
		s, ok := d.Web.Services[j.TargetID]
		var found bool
		v, found = d.Web.Versions[j.Version]
		if !ok || !found || s.OwnerID != j.OwnerID || s.DraftVersionID != j.Version || v.Version.Status != "pending" || v.AIReview.Token != j.Token {
			return ErrConflict
		}
		var err error
		credential, err = webResolveCredential(d, s.OwnerID, s.ID, j.Version, v.Version.Endpoint)
		return err
	}); err != nil {
		return nil, "", "technical_validation_failed"
	}
	if v.Version.Visibility != "public" {
		return nil, "", "private_listing"
	}
	quotes, err := legacyVersionCreditQuotes(j.TargetID, v)
	if err != nil {
		return nil, "", "technical_validation_failed"
	}
	remote, err := m.webRemoteCredential(ctx, v.Version.Endpoint, credential)
	if err != nil {
		return nil, "", "technical_validation_failed"
	}
	byName := map[string]Tool{}
	for _, tool := range remote {
		byName[tool.Name] = tool
	}
	for _, local := range v.Tools {
		t, ok := byName[local.Name]
		if !ok || !sameWebSchema(local.InputSchema, string(t.InputSchema)) || !sameWebSchema(local.OutputSchema, string(t.OutputSchema)) || digest(t) != v.RemoteDigests[local.ToolID] {
			return nil, "", "technical_validation_failed"
		}
	}
	ids := make([]string, 0, len(quotes))
	for _, quote := range quotes {
		ids = append(ids, quote.ID)
	}
	sort.Strings(ids)
	return ids, digest(v), ""
}

func (m *Module) WorkAIReviews(ctx context.Context, limit int) error {
	if m == nil || m.review == nil {
		return nil
	}
	if limit < 1 || limit > 20 {
		return ErrInvalid
	}
	for n := 0; n < limit; n++ {
		var claim *marketreview.Job
		err := m.store.update(func(d *database) error {
			w := webState(d)
			ids := make([]string, 0, len(w.Services))
			for id := range w.Services {
				ids = append(ids, id)
			}
			sort.Strings(ids)
			for _, id := range ids {
				s := w.Services[id]
				if s.Status == "deleted" {
					continue
				}
				for _, version := range []string{s.DraftVersionID, s.LiveVersionID} {
					v, ok := w.Versions[version]
					if !ok {
						continue
					}
					eligible := v.Version.Status == "pending"
					for _, job := range v.AIReview.Jobs {
						eligible = eligible || job.Token == v.AIReview.Token && job.ApprovalPending
					}
					if !eligible {
						continue
					}
					claim = v.AIReview.Claim(m.now().Unix())
					w.Versions[v.Version.ID] = v
					if claim != nil {
						break
					}
				}
				if claim != nil {
					break
				}
			}
			return nil
		})
		if err != nil {
			return err
		}
		if claim == nil {
			return nil
		}
		if claim.ApprovalPending {
			if err := m.recoverCoreApproval(ctx, *claim); err != nil {
				return err
			}
			continue
		}
		result, callErr := m.review.Classify(ctx, *claim)
		manual := ""
		var quotes []string
		var verified string
		if callErr == nil {
			quotes, verified, manual = m.reviewTechnical(ctx, *claim)
		}
		enabled, ownerErr := m.review.Classifier.OwnerEnabled(ctx, claim.OwnerID)
		if ownerErr != nil {
			enabled = true
			callErr = &marketreview.Failure{Code: "authority_unavailable", Temporary: true}
		}
		err = m.review.With(ctx, func(modes marketreview.Modes) error {
			var approvedRevision int64
			remoteAttempted := false
			if callErr == nil && manual == "" && enabled && !result.Flagged && claim.Mode == marketreview.Auto && modes.Tool == marketreview.Auto {
				port, ok := m.review.Classifier.(marketreview.ToolAuthority)
				if !ok || claim.AdmissionRevision < 1 {
					manual = "core_approval_required"
				} else {
					current := false
					if err := m.store.update(func(d *database) error {
						w := webState(d)
						s, ok := w.Services[claim.TargetID]
						v, found := w.Versions[claim.Version]
						current = ok && found && s.OwnerID == claim.OwnerID && s.Status != "deleted" && s.DraftVersionID == claim.Version && v.Version.Status == "pending" && v.AIReview.Token == claim.Token && digest(v) == verified
						if current {
							current = false
							for i := range v.AIReview.Jobs {
								j := &v.AIReview.Jobs[i]
								if j.ID == claim.ID && j.Token == claim.Token && j.Status == "running" && j.Attempts == claim.Attempts {
									j.ApprovalPending = true
									j.LeaseUntil = m.now().Unix() + 60
									claim.LeaseUntil = j.LeaseUntil
									current = true
									break
								}
							}
							if current {
								w.Versions[claim.Version] = v
								verified = digest(v)
							}
						}
						return nil
					}); err != nil {
						return err
					}
					if current {
						claim.ApprovalPending = true
						remoteAttempted = true
						var approveErr error
						approvedRevision, approveErr = port.ApproveTool(ctx, result.Receipt, quotes, claim.Token)
						if approveErr != nil {
							if err := m.revokeReviewAttempt(ctx, *claim); err != nil {
								return err
							}
							manual = "core_approval_failed"
							remoteAttempted = false
						}
					} else {
						manual = "stale_submission"
					}
				}
			}
			applied, staleApproval := false, false
			updateErr := m.store.update(func(d *database) error {
				w := webState(d)
				s, ok := w.Services[claim.TargetID]
				v, found := w.Versions[claim.Version]
				if !found {
					return nil
				}
				current := ok && s.Status != "deleted" && s.OwnerID == claim.OwnerID && s.DraftVersionID == claim.Version && v.Version.Status == "pending" && (verified == "" || digest(v) == verified)
				apply, approve := v.AIReview.Complete(*claim, result, callErr, modes.Tool, current, enabled, manual, m.now().Unix())
				if remoteAttempted && !apply {
					// Keep the durable pending marker until exact Core revocation
					// succeeds; a failed compensation must remain discoverable.
					staleApproval = true
					return ErrConflict
				}
				if apply {
					applied = true
					v.Version.Status = "rejected"
					if approve {
						v.Version.Status = "published"
						v.Version.PublishedAt = m.now().Unix()
						s.LiveVersionID = claim.Version
						s.DraftVersionID = ""
						s.CoreLifecycleRevision = approvedRevision
						if s.Status != "paused" {
							s.Status = "published"
						}
						// Keep the dispatch marker in the committed publication
						// until the local commit ACK is known. A commit may have
						// taken effect even when its acknowledgement is lost.
						for i := range v.AIReview.Jobs {
							j := &v.AIReview.Jobs[i]
							if j.ID == claim.ID && j.Token == claim.Token {
								j.ApprovalPending = true
								j.LeaseUntil = claim.LeaseUntil
							}
						}
					}
					v.Version.ReviewNote = ""
					v.Version.ReviewedBy = 0
					s.UpdatedAt = m.now().Unix()
					w.Services[s.ID] = s
				}
				w.Versions[v.Version.ID] = v
				return nil
			})
			if remoteAttempted && (updateErr != nil || !applied) {
				if err := m.recoverCoreApproval(ctx, *claim); err != nil {
					return err
				}
				if staleApproval {
					return nil
				}
			}
			if remoteAttempted && applied && updateErr == nil {
				return m.confirmCoreApproval(*claim)
			}
			return updateErr
		})
		if err != nil {
			return err
		}
	}
	return nil
}

// A persisted approval marker means Core may already have committed while Go
// has not. Exact-token revocation precedes the terminal local write. This path
// runs before mode, classifier and attempt-limit checks after a restart.
func (m *Module) recoverCoreApproval(ctx context.Context, claim marketreview.Job) error {
	if err := m.revokeReviewAttempt(ctx, claim); err != nil {
		return err
	}
	return m.store.update(func(d *database) error {
		w := webState(d)
		v, ok := w.Versions[claim.Version]
		if !ok || v.AIReview.Token != claim.Token {
			return nil
		}
		for _, job := range v.AIReview.Jobs {
			if job.ID == claim.ID && job.Token == claim.Token && job.Attempts == claim.Attempts && job.LeaseUntil == claim.LeaseUntil && job.ApprovalPending {
				s := w.Services[claim.TargetID]
				if job.Applied && s.LiveVersionID == claim.Version && s.DraftVersionID == "" {
					// The local publication committed but its ACK was lost.
					// Match Core's exact-token revocation by restoring the
					// same previously frozen live version locally.
					s.LiveVersionID = job.ApprovalPriorVersion
					s.DraftVersionID = claim.Version
					if s.LiveVersionID == "" {
						s.Status = "draft"
					} else if s.Status != "paused" {
						s.Status = "published"
					}
					s.UpdatedAt = m.now().Unix()
					v.Version.Status, v.Version.PublishedAt = "pending", 0
					w.Services[s.ID] = s
				}
				for i := range v.AIReview.Jobs {
					j := &v.AIReview.Jobs[i]
					if j.ID == claim.ID && j.Token == claim.Token {
						j.Status, j.Applied = "running", false
					}
				}
				v.AIReview.Manual("core_approval_uncertain", m.now().Unix())
				w.Versions[claim.Version] = v
				break
			}
		}
		return nil
	})
}

func (m *Module) confirmCoreApproval(claim marketreview.Job) error {
	return m.store.update(func(d *database) error {
		w := webState(d)
		v, ok := w.Versions[claim.Version]
		if !ok || v.AIReview.Token != claim.Token {
			return nil
		}
		for i := range v.AIReview.Jobs {
			j := &v.AIReview.Jobs[i]
			if j.ID == claim.ID && j.Token == claim.Token && j.Attempts == claim.Attempts && j.LeaseUntil == claim.LeaseUntil && j.Applied {
				j.ApprovalPending, j.LeaseUntil = false, 0
			}
		}
		w.Versions[claim.Version] = v
		return nil
	})
}

func (m *Module) revokeReviewAttempt(ctx context.Context, j marketreview.Job) error {
	port, ok := m.review.Classifier.(marketreview.ToolAuthority)
	if !ok {
		return ErrUnavailable
	}
	bounded, cancel := context.WithTimeout(context.WithoutCancel(ctx), 3*time.Second)
	defer cancel()
	if err := port.InvalidateTool(bounded, j.OwnerID, j.Token); err != nil {
		return ErrUnavailable
	}
	return nil
}
