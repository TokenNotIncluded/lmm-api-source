package checkins

import (
	"context"
	"database/sql"
	_ "embed"
	"encoding/json"
	"errors"
	"strconv"
	"time"
)

//go:embed schema.sql
var Schema string

type Service struct {
	db     *sql.DB
	ledger Ledger
}
type intention struct {
	Account, User, Amount, Revision, Created int64
	Day, Key, State                          string
	Level                                    uint32
	Record                                   []byte
}

func New(ctx context.Context, db *sql.DB, ledger Ledger) (*Service, error) {
	if db == nil || ledger == nil {
		return nil, ErrInvalid
	}
	var version int
	if e := db.QueryRowContext(ctx, "SELECT version FROM lmm_checkins.contract WHERE singleton").Scan(&version); e != nil || version != 1 {
		return nil, ErrUnavailable
	}
	if e := db.QueryRowContext(ctx, "SELECT version FROM lmm_checkins.legacy_history_contract WHERE singleton").Scan(&version); e != nil || version != 1 {
		return nil, ErrUnavailable
	}
	return &Service{db, ledger}, nil
}
func (s *Service) begin(ctx context.Context) (*sql.Tx, error) {
	tx, e := s.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
	if e != nil {
		return nil, ErrUnavailable
	}
	if _, e = tx.ExecContext(ctx, "SET LOCAL synchronous_commit=on; SET LOCAL statement_timeout='3s'; SET LOCAL lock_timeout='500ms'"); e != nil {
		tx.Rollback()
		return nil, ErrUnavailable
	}
	return tx, nil
}
func inspectValid(a Authority) bool {
	return a.UserID > 0 && a.AccountID > 0 && a.Timestamp > 0 && a.MaxDailyUnits >= 0 && a.Day == day(a.Timestamp)
}
func loadPolicy(ctx context.Context, tx *sql.Tx) (*Policy, *int64, error) {
	var revision int64
	var raw []byte
	e := tx.QueryRowContext(ctx, "SELECT p.revision,p.policy FROM lmm_checkins.active_policy a JOIN lmm_checkins.policy_revisions p ON p.revision=a.revision WHERE a.singleton").Scan(&revision, &raw)
	if errors.Is(e, sql.ErrNoRows) {
		return nil, nil, nil
	}
	if e != nil {
		return nil, nil, ErrUnavailable
	}
	var p Policy
	if json.Unmarshal(raw, &p) != nil || p.Validate() != nil {
		return nil, nil, ErrUnavailable
	}
	return &p, &revision, nil
}
func loadIntention(ctx context.Context, tx *sql.Tx, account int64, date string) (intention, error) {
	var i intention
	e := tx.QueryRowContext(ctx, "SELECT account_id,user_id,day::text,operation_key,amount_units,trust_level,policy_revision,created_at,state,record FROM lmm_checkins.intentions WHERE account_id=$1 AND day=$2::date", account, date).Scan(&i.Account, &i.User, &i.Day, &i.Key, &i.Amount, &i.Level, &i.Revision, &i.Created, &i.State, &i.Record)
	return i, e
}
func (s *Service) Status(ctx context.Context, user, month string) (Status, error) {
	a, e := s.ledger.Inspect(ctx, user)
	if e != nil {
		return Status{}, e
	}
	if !inspectValid(a) {
		return Status{}, ErrUnavailable
	}
	if month == "" {
		month = a.Day[:7]
	}
	if len(month) != 7 {
		return Status{}, ErrInvalid
	}
	if _, e = time.Parse("2006-01", month); e != nil || month[:4] == "0000" {
		return Status{}, ErrInvalid
	}
	tx, e := s.begin(ctx)
	if e != nil {
		return Status{}, e
	}
	defer tx.Rollback()
	p, revision, e := loadPolicy(ctx, tx)
	if e != nil {
		return Status{}, e
	}
	result := Status{Today: a.Day, Month: month, CalendarOffsetSeconds: Offset, PolicyRevision: revision, Records: []DailyRecord{}, TotalQuotaAwarded: "0"}
	if p != nil && p.Enabled {
		r, e := p.rewardRange()
		if e != nil {
			return Status{}, e
		}
		maxUnits, _ := strconv.ParseInt(r.MaxQuota, 10, 64)
		if maxUnits > a.MaxDailyUnits {
			return Status{}, ErrUnavailable
		}
		result.Enabled = !a.Blocked
		result.RewardRange = &r
	}
	var ownerMismatch bool
	if tx.QueryRowContext(ctx, `SELECT EXISTS(SELECT FROM lmm_checkins.legacy_history
 WHERE (target_user_id=$1 OR target_personal_account_id=$2)
 AND (target_user_id<>$1 OR target_personal_account_id<>$2))`, a.UserID, a.AccountID).Scan(&ownerMismatch) != nil || ownerMismatch {
		return Status{}, ErrUnavailable
	}
	rows, e := tx.QueryContext(ctx, `SELECT record FROM (
 SELECT day,record FROM lmm_checkins.intentions WHERE account_id=$1 AND state='settled'
 UNION ALL SELECT day,`+legacyDailyRecordSQL+` FROM lmm_checkins.legacy_history WHERE target_personal_account_id=$1 AND target_user_id=$3
 ) records WHERE left(day::text,7)=$2 ORDER BY day DESC`, a.AccountID, month, a.UserID)
	if e != nil {
		return Status{}, ErrUnavailable
	}
	for rows.Next() {
		var raw []byte
		var record DailyRecord
		if rows.Scan(&raw) != nil || json.Unmarshal(raw, &record) != nil {
			rows.Close()
			return Status{}, ErrUnavailable
		}
		result.Records = append(result.Records, record)
	}
	e = rows.Err()
	rows.Close()
	if e != nil {
		return Status{}, ErrUnavailable
	}
	i, e := loadIntention(ctx, tx, a.AccountID, a.Day)
	if e == nil {
		result.Pending = i.State == "pending"
		if i.State == "settled" {
			var record DailyRecord
			if json.Unmarshal(i.Record, &record) != nil {
				return Status{}, ErrUnavailable
			}
			result.TodayRecord = &record
			result.CheckedInToday = true
		}
	} else if !errors.Is(e, sql.ErrNoRows) {
		return Status{}, ErrUnavailable
	}
	historical, historyError := loadLegacyHistory(ctx, tx, a.UserID, a.AccountID, a.Day)
	if historyError != nil || historical != nil && e == nil {
		return Status{}, ErrUnavailable
	}
	if historical != nil {
		result.TodayRecord = historical
		result.CheckedInToday = true
	}
	if tx.QueryRowContext(ctx, `SELECT count(*)::bigint,coalesce(sum(amount_units::numeric),0)::text FROM (
 SELECT amount_units FROM lmm_checkins.intentions WHERE account_id=$1 AND state='settled'
 UNION ALL SELECT amount_units FROM lmm_checkins.legacy_history WHERE target_personal_account_id=$1 AND target_user_id=$2
 ) records`, a.AccountID, a.UserID).Scan(&result.TotalCheckins, &result.TotalQuotaAwarded) != nil {
		return Status{}, ErrUnavailable
	}
	if tx.Commit() != nil {
		return Status{}, ErrUnavailable
	}
	return result, nil
}

// Challenge preserves the canonical operation of a pending daily intention.
// A browser reload may suggest a new key, but it cannot redraw or replace it.
func (s *Service) Challenge(ctx context.Context, user, key string) (Challenge, error) {
	if !validKey(key) {
		return Challenge{}, ErrInvalid
	}
	a, e := s.ledger.Inspect(ctx, user)
	if e != nil {
		return Challenge{}, e
	}
	if !inspectValid(a) {
		return Challenge{}, ErrUnavailable
	}
	if a.Blocked {
		return Challenge{}, ErrForbidden
	}
	tx, e := s.begin(ctx)
	if e != nil {
		return Challenge{}, e
	}
	defer tx.Rollback()
	historical, e := loadLegacyHistory(ctx, tx, a.UserID, a.AccountID, a.Day)
	if e != nil {
		return Challenge{}, e
	}
	if historical != nil {
		return Challenge{}, ErrConflict
	}
	var commandDay string
	e = tx.QueryRowContext(ctx, "SELECT day::text FROM lmm_checkins.claim_commands WHERE user_id=$1 AND operation_key=$2", a.UserID, key).Scan(&commandDay)
	if e == nil && commandDay != a.Day {
		// An old operation remains usable for receipt recovery, but a new day's
		// proof must not be bound to an immutable command from a previous day.
		return Challenge{}, ErrConflict
	}
	if e != nil && !errors.Is(e, sql.ErrNoRows) {
		return Challenge{}, ErrUnavailable
	}
	i, e := loadIntention(ctx, tx, a.AccountID, a.Day)
	if e == nil {
		if i.User != a.UserID || i.State == "settled" {
			return Challenge{}, ErrConflict
		}
		key = i.Key
	} else if !errors.Is(e, sql.ErrNoRows) {
		return Challenge{}, ErrUnavailable
	} else {
		p, _, e := loadPolicy(ctx, tx)
		if e != nil {
			return Challenge{}, e
		}
		if p == nil || !p.Enabled {
			return Challenge{}, ErrForbidden
		}
		r, e := p.rewardRange()
		if e != nil {
			return Challenge{}, e
		}
		high, _ := strconv.ParseInt(r.MaxQuota, 10, 64)
		if high > a.MaxDailyUnits {
			return Challenge{}, ErrUnavailable
		}
	}
	if tx.Commit() != nil {
		return Challenge{}, ErrUnavailable
	}
	challenge, e := s.ledger.Challenge(ctx, user, key)
	if e != nil {
		return Challenge{}, e
	}
	if challenge.ChallengeID == "" || challenge.OperationKey != key || challenge.Day != a.Day || challenge.ExpiresAt <= a.Timestamp || challenge.TurnstileSiteKey == "" || challenge.TurnstileAction != "daily_checkin" || challenge.TurnstileCData == "" {
		return Challenge{}, ErrUnavailable
	}
	if challenge.PasskeyRequired && (len(challenge.PasskeyOptions) == 0 || challenge.PasskeyOptions[0] != '{' || !json.Valid(challenge.PasskeyOptions)) {
		return Challenge{}, ErrUnavailable
	}
	return challenge, nil
}

// A pending intention is committed before the financial RPC. UNKNOWN is never
// treated as a failed award or resolved using a new amount/financial key.
func (s *Service) Claim(ctx context.Context, user string, input ClaimRequest) (ClaimResult, error) {
	if !validKey(input.OperationKey) {
		return ClaimResult{}, ErrInvalid
	}
	a, e := s.ledger.Inspect(ctx, user)
	if e != nil {
		return ClaimResult{}, e
	}
	if !inspectValid(a) {
		return ClaimResult{}, ErrUnavailable
	}
	i, prior, e := s.prepareClaim(ctx, a, input.OperationKey, false)
	authorization := ""
	if e == ErrVerificationRequired {
		grant, verifyError := s.authorize(ctx, user, input, input.OperationKey, a.Day)
		if verifyError != nil {
			return ClaimResult{}, verifyError
		}
		authorization = grant.ID
		// Verification performs external I/O without holding a database transaction.
		// Recheck identity and day before freezing a monetary intention.
		a, e = s.ledger.Inspect(ctx, user)
		if e != nil {
			return ClaimResult{}, e
		}
		if !inspectValid(a) {
			return ClaimResult{}, ErrUnavailable
		}
		if a.Day != grant.Day {
			return ClaimResult{}, ErrVerificationRequired
		}
		i, prior, e = s.prepareClaim(ctx, a, input.OperationKey, true)
	}
	if e != nil {
		return ClaimResult{}, e
	}
	if prior != nil {
		return *prior, nil
	}
	if i.State == "settled" {
		var record DailyRecord
		if json.Unmarshal(i.Record, &record) != nil {
			return ClaimResult{}, ErrUnavailable
		}
		return s.remember(ctx, a.UserID, input.OperationKey, ClaimResult{AlreadyClaimed: i.Key != input.OperationKey, Record: record}, i.Record)
	}
	if i.Key != input.OperationKey {
		authorization = ""
	}
	posting := Posting{Day: i.Day, Amount: i.Amount, Level: i.Level, OperationKey: i.Key, AuthorizationID: authorization}
	// A pending result may already exist in Core. Try the original intent before
	// requesting a new challenge, so expired proofs do not hide committed funds.
	receipt, e := s.ledger.Post(ctx, user, posting)
	if e == ErrVerificationRequired && authorization == "" {
		grant, verifyError := s.authorize(ctx, user, input, i.Key, i.Day)
		if verifyError != nil {
			return ClaimResult{}, verifyError
		}
		posting.AuthorizationID = grant.ID
		receipt, e = s.ledger.Post(ctx, user, posting)
	}
	if e != nil {
		return ClaimResult{}, e
	}
	if receipt.UserID != i.User || receipt.AccountID != i.Account || receipt.Day != i.Day || receipt.AmountUnits != i.Amount || receipt.Timestamp <= 0 || (receipt.JournalID != nil && *receipt.JournalID <= 0) || (i.Amount == 0) != (receipt.JournalID == nil) {
		return ClaimResult{}, ErrConflict
	}
	record := DailyRecord{CheckinDate: i.Day, QuotaAwarded: strconv.FormatInt(i.Amount, 10), CreatedAt: receipt.Timestamp, JournalID: receipt.JournalID, Origin: "native", PolicyRevision: &i.Revision}
	// Preserve the immutable storage contract without exposing the historical
	// scalar in the public DailyRecord DTO. New intentions always store zero.
	raw, e := json.Marshal(struct {
		DailyRecord
		LegacyLevel uint32 `json:"trust_level"`
	}{record, i.Level})
	if e != nil {
		return ClaimResult{}, ErrUnavailable
	}
	finish, e := s.begin(ctx)
	if e != nil {
		return ClaimResult{}, e
	}
	defer finish.Rollback()
	if _, e = finish.ExecContext(ctx, "UPDATE lmm_checkins.intentions SET state='settled',record=$1::jsonb WHERE account_id=$2 AND day=$3::date AND state='pending'", string(raw), i.Account, i.Day); e != nil {
		return ClaimResult{}, ErrUnavailable
	}
	actual, e := loadIntention(ctx, finish, i.Account, i.Day)
	if e != nil || actual.State != "settled" || json.Unmarshal(actual.Record, &record) != nil {
		return ClaimResult{}, ErrUnavailable
	}
	result := ClaimResult{AlreadyClaimed: i.Key != input.OperationKey, Record: record}
	resultRaw, _ := storedClaimResult(result, actual.Record)
	if _, e = finish.ExecContext(ctx, "UPDATE lmm_checkins.claim_commands SET result=$1::jsonb WHERE user_id=$2 AND operation_key=$3 AND result IS NULL", string(resultRaw), a.UserID, input.OperationKey); e != nil {
		return ClaimResult{}, ErrUnavailable
	}
	if finish.Commit() != nil {
		return ClaimResult{}, ErrUnavailable
	}
	return result, nil
}

// prepareClaim checks durable state and commits only when a first intention has
// already received Core authorization. Its transaction never spans verification.
func (s *Service) prepareClaim(ctx context.Context, a Authority, key string, authorized bool) (intention, *ClaimResult, error) {
	tx, e := s.begin(ctx)
	if e != nil {
		return intention{}, nil, e
	}
	defer tx.Rollback()
	if _, e = tx.ExecContext(ctx, "SELECT pg_advisory_xact_lock(hashtextextended('lmm-checkins:'||$1::text,0))", a.AccountID); e != nil {
		return intention{}, nil, ErrUnavailable
	}
	date := a.Day
	var prior []byte
	e = tx.QueryRowContext(ctx, "SELECT day::text,result FROM lmm_checkins.claim_commands WHERE user_id=$1 AND operation_key=$2", a.UserID, key).Scan(&date, &prior)
	if e == nil && prior != nil {
		var result ClaimResult
		if json.Unmarshal(prior, &result) != nil {
			return intention{}, nil, ErrUnavailable
		}
		if tx.Commit() != nil {
			return intention{}, nil, ErrUnavailable
		}
		return intention{}, &result, nil
	}
	if e != nil && !errors.Is(e, sql.ErrNoRows) {
		return intention{}, nil, ErrUnavailable
	}
	commandExists := e == nil
	historical, e := loadLegacyHistory(ctx, tx, a.UserID, a.AccountID, date)
	if e != nil {
		return intention{}, nil, e
	}
	if historical != nil {
		if commandExists {
			return intention{}, nil, ErrConflict
		}
		if tx.Commit() != nil {
			return intention{}, nil, ErrUnavailable
		}
		return intention{}, &ClaimResult{AlreadyClaimed: true, Record: *historical}, nil
	}
	i, e := loadIntention(ctx, tx, a.AccountID, date)
	if errors.Is(e, sql.ErrNoRows) {
		if commandExists {
			return intention{}, nil, ErrConflict
		}
		if a.Blocked {
			return intention{}, nil, ErrForbidden
		}
		p, revision, e := loadPolicy(ctx, tx)
		if e != nil {
			return intention{}, nil, e
		}
		if p == nil || !p.Enabled {
			return intention{}, nil, ErrForbidden
		}
		r, e := p.rewardRange()
		if e != nil {
			return intention{}, nil, e
		}
		high, _ := strconv.ParseInt(r.MaxQuota, 10, 64)
		if high > a.MaxDailyUnits {
			return intention{}, nil, ErrUnavailable
		}
		if !authorized {
			return intention{}, nil, ErrVerificationRequired
		}
		amount, e := draw(r)
		if e != nil {
			return intention{}, nil, e
		}
		i = intention{Account: a.AccountID, User: a.UserID, Day: date, Key: key, Amount: amount, Level: 0, Revision: *revision, Created: a.Timestamp, State: "pending"}
		if _, e = tx.ExecContext(ctx, "INSERT INTO lmm_checkins.intentions(account_id,user_id,day,operation_key,amount_units,trust_level,policy_revision,created_at,state) VALUES($1,$2,$3::date,$4,$5,$6,$7,$8,'pending')", i.Account, i.User, i.Day, i.Key, i.Amount, i.Level, i.Revision, i.Created); e != nil {
			return intention{}, nil, ErrUnavailable
		}
	} else if e != nil {
		return intention{}, nil, ErrUnavailable
	}
	if i.User != a.UserID {
		return intention{}, nil, ErrConflict
	}
	if !commandExists {
		if _, e = tx.ExecContext(ctx, "INSERT INTO lmm_checkins.claim_commands(user_id,operation_key,account_id,day) VALUES($1,$2,$3,$4::date)", a.UserID, key, a.AccountID, date); e != nil {
			return intention{}, nil, ErrUnavailable
		}
	}
	if tx.Commit() != nil {
		return intention{}, nil, ErrUnavailable
	}
	return i, nil, nil
}

func (s *Service) authorize(ctx context.Context, user string, input ClaimRequest, key, date string) (Authorization, error) {
	if input.ChallengeID == "" || input.TurnstileToken == "" {
		return Authorization{}, ErrVerificationRequired
	}
	input.OperationKey = key
	grant, e := s.ledger.Authorize(ctx, user, input)
	if e != nil {
		return Authorization{}, e
	}
	if grant.ID == "" || grant.Day != date || grant.ExpiresAt <= 0 {
		return Authorization{}, ErrUnavailable
	}
	return grant, nil
}

func (s *Service) remember(ctx context.Context, user int64, key string, result ClaimResult, storedRecord json.RawMessage) (ClaimResult, error) {
	raw, _ := storedClaimResult(result, storedRecord)
	if _, e := s.db.ExecContext(ctx, "UPDATE lmm_checkins.claim_commands SET result=$1::jsonb WHERE user_id=$2 AND operation_key=$3 AND result IS NULL", string(raw), user, key); e != nil {
		return ClaimResult{}, ErrUnavailable
	}
	return result, nil
}

// Persist the original record bytes, including historical fields required by
// the immutable ledger trigger. Public responses use the narrower DailyRecord.
func storedClaimResult(result ClaimResult, record json.RawMessage) ([]byte, error) {
	return json.Marshal(struct {
		AlreadyClaimed bool            `json:"already_claimed"`
		Record         json.RawMessage `json:"record"`
	}{result.AlreadyClaimed, record})
}
