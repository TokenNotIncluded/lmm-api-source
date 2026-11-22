package checkins

import (
	"context"
	"database/sql"
	"strconv"
)

// PublishedConfiguration contains only the currently published public reward
// policy. It is not a user's eligibility, verification, or funding authority.
type PublishedConfiguration struct {
	Revision   int64
	Enabled    bool
	MinCredits int64
	MaxCredits int64
}

func publicConfiguration(p *Policy, revision *int64) (PublishedConfiguration, error) {
	if p == nil && revision == nil {
		return PublishedConfiguration{}, nil
	}
	if p == nil || revision == nil || *revision <= 0 || p.Validate() != nil {
		return PublishedConfiguration{}, ErrUnavailable
	}
	r, err := p.rewardRange()
	if err != nil {
		return PublishedConfiguration{}, ErrUnavailable
	}
	low, e1 := strconv.ParseInt(r.MinQuota, 10, 64)
	high, e2 := strconv.ParseInt(r.MaxQuota, 10, 64)
	if e1 != nil || e2 != nil {
		return PublishedConfiguration{}, ErrUnavailable
	}
	return PublishedConfiguration{Revision: *revision, Enabled: p.Enabled, MinCredits: low, MaxCredits: high}, nil
}

// PublicConfiguration reads the same validated policy used by Status. It does
// not touch intentions, challenges, historical rewards, or the Core ledger.
func (s *Service) PublicConfiguration(ctx context.Context) (PublishedConfiguration, error) {
	tx, err := s.db.BeginTx(ctx, &sql.TxOptions{ReadOnly: true})
	if err != nil {
		return PublishedConfiguration{}, ErrUnavailable
	}
	defer tx.Rollback()
	p, revision, err := loadPolicy(ctx, tx)
	if err != nil {
		return PublishedConfiguration{}, err
	}
	result, err := publicConfiguration(p, revision)
	if err != nil {
		return PublishedConfiguration{}, err
	}
	if tx.Commit() != nil {
		return PublishedConfiguration{}, ErrUnavailable
	}
	return result, nil
}
