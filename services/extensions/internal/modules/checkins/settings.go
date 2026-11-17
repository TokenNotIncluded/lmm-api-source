// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package checkins

import (
	"context"
	"database/sql"
	_ "embed"
	"encoding/json"
	"errors"
	"fmt"
	"math"
	"strconv"

	"github.com/lib/pq"
)

//go:embed settings.sql
var SettingsSchema string

const settingsFunction = "lmm_checkins.save_settings(bigint,text,boolean,text,text)"

type Settings struct {
	Revision         int64   `json:"revision"`
	Enabled          bool    `json:"enabled"`
	MinQuota         string  `json:"min_quota"`
	MaxQuota         string  `json:"max_quota"`
	Writable         bool    `json:"writable"`
	FundingAvailable bool    `json:"funding_available"`
	FundingLimit     *string `json:"funding_limit"`
}

type SettingsInput struct {
	Revision     int64  `json:"revision"`
	OperationKey string `json:"operation_key"`
	Enabled      bool   `json:"enabled"`
	MinQuota     string `json:"min_quota"`
	MaxQuota     string `json:"max_quota"`
}

func (input SettingsInput) validate() error {
	low, e1 := strconv.ParseInt(input.MinQuota, 10, 64)
	high, e2 := strconv.ParseInt(input.MaxQuota, 10, 64)
	if input.Revision < 0 || input.Revision == math.MaxInt64 || !validKey(input.OperationKey) || len(input.OperationKey) > 40 ||
		e1 != nil || e2 != nil || low < 0 || high < low ||
		strconv.FormatInt(low, 10) != input.MinQuota || strconv.FormatInt(high, 10) != input.MaxQuota {
		return ErrInvalid
	}
	for _, c := range input.OperationKey {
		if !(c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z' || c >= '0' && c <= '9' || c == '_' || c == '-') {
			return ErrInvalid
		}
	}
	return nil
}

// A missing addon is read-only; ordinary service startup never installs it.
func settingsWritable(ctx context.Context, tx *sql.Tx) (bool, error) {
	var writable bool
	e := tx.QueryRowContext(ctx, `SELECT coalesce((
 SELECT p.prosecdef AND p.proowner=d.datdba AND p.proconfig=ARRAY['search_path=pg_catalog']
 AND has_function_privilege(current_user,p.oid,'EXECUTE')
 AND NOT EXISTS(SELECT FROM aclexplode(coalesce(p.proacl,acldefault('f',p.proowner))) a WHERE a.grantee=0 AND a.privilege_type='EXECUTE')
 FROM pg_proc p JOIN pg_database d ON d.datname=current_database()
 WHERE p.oid=to_regprocedure($1)),false)`, settingsFunction).Scan(&writable)
	if e != nil {
		return false, ErrUnavailable
	}
	return writable, nil
}

func settingsFromPolicy(p *Policy, revision *int64) Settings {
	result := Settings{MinQuota: "0", MaxQuota: "0"}
	if p != nil && revision != nil {
		result.Revision = *revision
		result.Enabled = p.Enabled
		result.MinQuota = p.MinQuota
		result.MaxQuota = p.MaxQuota
	}
	return result
}

// Funding discovery describes Core's current ceiling; it does not create a
// challenge, credit a balance or claim that verification is configured.
func (s *Service) funding(ctx context.Context, user string, result *Settings) {
	a, e := s.ledger.Inspect(ctx, user)
	if e != nil || !inspectValid(a) {
		return
	}
	limit := strconv.FormatInt(a.MaxDailyUnits, 10)
	result.FundingLimit = &limit
	result.FundingAvailable = true
}

// Callers must freshly authorize a platform administrator before these methods.
func (s *Service) Settings(ctx context.Context, user string) (Settings, error) {
	tx, e := s.begin(ctx)
	if e != nil {
		return Settings{}, e
	}
	defer tx.Rollback()
	p, revision, e := loadPolicy(ctx, tx)
	if e != nil {
		return Settings{}, e
	}
	result := settingsFromPolicy(p, revision)
	result.Writable, e = settingsWritable(ctx, tx)
	if e != nil {
		return Settings{}, e
	}
	if tx.Commit() != nil {
		return Settings{}, ErrUnavailable
	}
	s.funding(ctx, user, &result)
	return result, nil
}

func (s *Service) SaveSettings(ctx context.Context, user string, actor int64, input SettingsInput) (Settings, error) {
	if actor <= 0 || input.validate() != nil {
		return Settings{}, ErrInvalid
	}
	result := Settings{}
	s.funding(ctx, user, &result)
	if result.FundingLimit != nil && input.Enabled {
		limit, _ := strconv.ParseInt(*result.FundingLimit, 10, 64)
		maximum, _ := strconv.ParseInt(input.MaxQuota, 10, 64)
		if maximum > limit {
			return Settings{}, ErrInvalid
		}
	}
	tx, e := s.begin(ctx)
	if e != nil {
		return Settings{}, e
	}
	defer tx.Rollback()
	writable, e := settingsWritable(ctx, tx)
	if e != nil {
		return Settings{}, e
	}
	if !writable {
		return Settings{}, ErrUnavailable
	}
	operation := fmt.Sprintf("admin-%d-%s", actor, input.OperationKey)
	var revision int64
	e = tx.QueryRowContext(ctx, "SELECT lmm_checkins.save_settings($1,$2,$3,$4,$5)", input.Revision, operation, input.Enabled, input.MinQuota, input.MaxQuota).Scan(&revision)
	if e != nil {
		var databaseError *pq.Error
		if errors.As(e, &databaseError) {
			if databaseError.Code == "40001" {
				return Settings{}, ErrConflict
			}
			if databaseError.Code.Class() == "22" {
				return Settings{}, ErrInvalid
			}
		}
		return Settings{}, ErrUnavailable
	}
	var raw []byte
	if tx.QueryRowContext(ctx, "SELECT policy FROM lmm_checkins.policy_revisions WHERE revision=$1", revision).Scan(&raw) != nil {
		return Settings{}, ErrUnavailable
	}
	var policy Policy
	if json.Unmarshal(raw, &policy) != nil || policy.Validate() != nil {
		return Settings{}, ErrUnavailable
	}
	saved := settingsFromPolicy(&policy, &revision)
	saved.Writable = true
	saved.FundingAvailable = result.FundingAvailable
	saved.FundingLimit = result.FundingLimit
	if tx.Commit() != nil {
		return Settings{}, ErrUnavailable
	}
	return saved, nil
}
