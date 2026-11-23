// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
package checkins

import (
	"bytes"
	"context"
	"crypto/sha256"
	"database/sql"
	_ "embed"
	"encoding/hex"
	"encoding/json"
	"errors"
	"io"
	"regexp"
	"time"

	"github.com/lib/pq"
)

//go:embed legacy_history.sql
var LegacyHistorySchema string

const LegacyHistoryScope = "checkin-runtime-legacy-history-v1"

var historyDigest = regexp.MustCompile(`^[0-9a-f]{64}$`)

type LegacyHistoryOwner struct {
	SourceUserID            int64 `json:"source_user_id"`
	TargetPersonalAccountID int64 `json:"target_personal_account_id"`
	TargetUserID            int64 `json:"target_user_id"`
}

type LegacyHistoryRecord struct {
	ID           int64  `json:"id"`
	UserID       int64  `json:"user_id"`
	CheckinDate  string `json:"checkin_date"`
	QuotaAwarded int64  `json:"quota_awarded"`
	CreatedAt    int64  `json:"created_at"`
}

// The offline preparation tool must validate the actual committed Core receipt
// and complete source identities before producing this input. The extension
// has no Core database credential and does not substitute its own authority.
// The operator pins the exact complete input bytes with --input-sha256.
type LegacyHistoryInput struct {
	Scope                 string                `json:"scope"`
	SourceNamespace       string                `json:"source_namespace"`
	SourceArchiveSHA256   string                `json:"source_archive_sha256"`
	SourceExportSHA256    string                `json:"source_export_sha256"`
	IdentityReceiptSHA256 string                `json:"identity_receipt_sha256"`
	IdentityMappingSHA256 string                `json:"identity_mapping_sha256"`
	CalendarTimezone      string                `json:"calendar_timezone"`
	CalendarOffsetSeconds int                   `json:"calendar_offset_seconds"`
	ObservedAt            int64                 `json:"observed_at"`
	SourceRowCount        int64                 `json:"source_row_count"`
	SourceUserIDs         []int64               `json:"source_user_ids"`
	Mapping               []LegacyHistoryOwner  `json:"mapping"`
	Rows                  []LegacyHistoryRecord `json:"rows"`
}

type LegacyHistoryReceipt struct {
	OperationID           string `json:"operation_id"`
	InputSHA256           string `json:"input_sha256"`
	SourceNamespaceSHA256 string `json:"source_namespace_sha256"`
	SourceRows            int64  `json:"source_rows"`
	MappedRows            int64  `json:"mapped_rows"`
	ArchivedRows          int64  `json:"archived_rows"`
	AddedRows             int64  `json:"added_rows"`
}

type legacyHistoryRow struct {
	SourceID                int64  `json:"source_id"`
	SourceUserID            int64  `json:"source_user_id"`
	Day                     string `json:"day"`
	AmountUnits             int64  `json:"amount_units"`
	CreatedAt               int64  `json:"created_at"`
	TargetUserID            *int64 `json:"target_user_id"`
	TargetPersonalAccountID *int64 `json:"target_personal_account_id"`
	SourceRowSHA256         string `json:"source_row_sha256"`
}

func digestHistory(raw []byte) string {
	h := sha256.Sum256(raw)
	return hex.EncodeToString(h[:])
}

func decodeLegacyHistory(raw []byte, expected string) (LegacyHistoryInput, []legacyHistoryRow, error) {
	var input LegacyHistoryInput
	if len(raw) == 0 || len(raw) > 16*1024*1024 || !historyDigest.MatchString(expected) || digestHistory(raw) != expected {
		return input, nil, ErrInvalid
	}
	d := json.NewDecoder(bytes.NewReader(raw))
	d.DisallowUnknownFields()
	var extra any
	if d.Decode(&input) != nil || d.Decode(&extra) != io.EOF || input.Scope != LegacyHistoryScope ||
		input.SourceNamespace == "" || len(input.SourceNamespace) > 200 || input.CalendarTimezone != "Asia/Shanghai" ||
		input.CalendarOffsetSeconds != Offset || input.ObservedAt <= 0 || input.ObservedAt > 9007199254740991 || input.SourceRowCount != int64(len(input.Rows)) ||
		input.SourceUserIDs == nil || input.Mapping == nil || input.Rows == nil || len(input.SourceUserIDs) != len(input.Mapping) {
		return input, nil, ErrInvalid
	}
	for _, c := range input.SourceNamespace {
		if c < '!' || c > '~' {
			return input, nil, ErrInvalid
		}
	}
	for _, hash := range []string{input.SourceArchiveSHA256, input.SourceExportSHA256, input.IdentityReceiptSHA256, input.IdentityMappingSHA256} {
		if !historyDigest.MatchString(hash) {
			return input, nil, ErrInvalid
		}
	}
	// Struct fields intentionally use canonical key order; all values are exact
	// integers. This is SHA256 of sorted-key compact JSON, without a trailing LF.
	mappingRaw, _ := json.Marshal(input.Mapping)
	if digestHistory(mappingRaw) != input.IdentityMappingSHA256 {
		return input, nil, ErrInvalid
	}
	owners := make(map[int64]LegacyHistoryOwner, len(input.Mapping))
	users, accounts := map[int64]bool{}, map[int64]bool{}
	for n, owner := range input.Mapping {
		if owner.SourceUserID <= 0 || owner.TargetUserID <= 0 || owner.TargetPersonalAccountID <= 0 ||
			owner.SourceUserID != input.SourceUserIDs[n] || n > 0 && input.SourceUserIDs[n-1] >= owner.SourceUserID ||
			users[owner.TargetUserID] || accounts[owner.TargetPersonalAccountID] {
			return input, nil, ErrInvalid
		}
		owners[owner.SourceUserID] = owner
		users[owner.TargetUserID], accounts[owner.TargetPersonalAccountID] = true, true
	}
	rows := make([]legacyHistoryRow, 0, len(input.Rows))
	type ownerDay struct {
		user int64
		day  string
	}
	days := map[ownerDay]bool{}
	for n, record := range input.Rows {
		date, e := time.Parse("2006-01-02", record.CheckinDate)
		key := ownerDay{record.UserID, record.CheckinDate}
		if record.ID <= 0 || record.UserID <= 0 || record.QuotaAwarded < 0 || record.CreatedAt < 0 || record.CreatedAt > input.ObservedAt ||
			e != nil || date.Year() < 1 || date.Format("2006-01-02") != record.CheckinDate ||
			n > 0 && input.Rows[n-1].ID >= record.ID || days[key] {
			return input, nil, ErrInvalid
		}
		days[key] = true
		canonical, _ := json.Marshal(map[string]any{"id": record.ID, "user_id": record.UserID, "checkin_date": record.CheckinDate, "quota_awarded": record.QuotaAwarded, "created_at": record.CreatedAt})
		row := legacyHistoryRow{SourceID: record.ID, SourceUserID: record.UserID, Day: record.CheckinDate, AmountUnits: record.QuotaAwarded, CreatedAt: record.CreatedAt, SourceRowSHA256: digestHistory(canonical)}
		if owner, ok := owners[record.UserID]; ok {
			row.TargetUserID, row.TargetPersonalAccountID = &owner.TargetUserID, &owner.TargetPersonalAccountID
		}
		rows = append(rows, row)
	}
	return input, rows, nil
}

const legacyHistoryRowsSQL = `SELECT * FROM jsonb_to_recordset($1::jsonb) AS r(
 source_id bigint,source_user_id bigint,day date,amount_units bigint,created_at bigint,
 target_user_id bigint,target_personal_account_id bigint,source_row_sha256 text)`

const legacyHistoryMismatchSQL = `h.source_user_id IS DISTINCT FROM r.source_user_id OR h.day IS DISTINCT FROM r.day
 OR h.amount_units IS DISTINCT FROM r.amount_units OR h.created_at IS DISTINCT FROM r.created_at
 OR h.target_user_id IS DISTINCT FROM r.target_user_id OR h.target_personal_account_id IS DISTINCT FROM r.target_personal_account_id
 OR h.source_row_sha256 IS DISTINCT FROM r.source_row_sha256`

// ImportLegacyHistory runs within a caller-owned SERIALIZABLE transaction under
// the fenced NOLOGIN database owner. DDL, data and the original recovery receipt
// commit together. No policy, native intention or Core balance is changed.
func ImportLegacyHistory(ctx context.Context, tx *sql.Tx, runtime, operation string, raw []byte, expected string, recoverOnly bool) (LegacyHistoryReceipt, error) {
	input, rows, e := decodeLegacyHistory(raw, expected)
	if e != nil || !validKey(operation) || runtime == "" {
		return LegacyHistoryReceipt{}, ErrInvalid
	}
	result := LegacyHistoryReceipt{OperationID: operation, InputSHA256: expected, SourceNamespaceSHA256: digestHistory([]byte(input.SourceNamespace)), SourceRows: int64(len(rows))}
	for _, row := range rows {
		if row.TargetUserID == nil {
			result.ArchivedRows++
		} else {
			result.MappedRows++
		}
	}
	var allowed bool
	e = tx.QueryRowContext(ctx, `SELECT current_user=pg_get_userbyid(d.datdba)
 AND current_setting('transaction_isolation')='serializable'
 AND EXISTS(SELECT FROM pg_roles WHERE rolname=current_user AND NOT rolcanlogin)
 AND EXISTS(SELECT FROM pg_roles WHERE rolname=$1 AND NOT rolcanlogin)
 AND session_user<>$1
 AND NOT EXISTS(SELECT FROM pg_roles WHERE rolname IN(session_user,current_user,$1) AND (rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls))
 AND NOT EXISTS(SELECT FROM pg_auth_members WHERE member=(SELECT oid FROM pg_roles WHERE rolname=$1))
 AND NOT EXISTS(SELECT FROM pg_roles WHERE rolcanlogin AND NOT rolsuper AND rolname<>session_user
   AND (has_database_privilege(oid,d.oid,'CONNECT') OR pg_has_role(oid,current_user,'SET') OR pg_has_role(oid,$1,'SET')))
 AND NOT EXISTS(SELECT FROM pg_stat_activity WHERE datid=d.oid AND pid<>pg_backend_pid())
 AND NOT EXISTS(SELECT FROM pg_prepared_xacts WHERE database=current_database())
 AND NOT EXISTS(SELECT FROM pg_namespace WHERE nspname='lmm_checkins' AND nspowner<>d.datdba)
 FROM pg_database d WHERE datname=current_database()`, runtime).Scan(&allowed)
	if e != nil || !allowed {
		return result, ErrUnavailable
	}
	if _, e = tx.ExecContext(ctx, `SET LOCAL synchronous_commit=on;
 SELECT pg_advisory_xact_lock(hashtextextended('lmm-checkins-owner',0));
 LOCK TABLE lmm_checkins.contract,lmm_checkins.policy_revisions,lmm_checkins.active_policy,lmm_checkins.intentions,lmm_checkins.claim_commands IN ACCESS EXCLUSIVE MODE`); e != nil {
		return result, ErrUnavailable
	}
	var version int
	if tx.QueryRowContext(ctx, `SELECT version FROM lmm_checkins.contract WHERE singleton`).Scan(&version) != nil || version != 1 {
		return result, ErrUnavailable
	}
	var installed, records, receipts bool
	if tx.QueryRowContext(ctx, `SELECT to_regclass('lmm_checkins.legacy_history_contract') IS NOT NULL,to_regclass('lmm_checkins.legacy_history') IS NOT NULL,to_regclass('lmm_checkins.legacy_history_imports') IS NOT NULL`).Scan(&installed, &records, &receipts) != nil {
		return result, ErrUnavailable
	}
	if !installed {
		if records || receipts || recoverOnly {
			return result, ErrConflict
		}
		if _, e = tx.ExecContext(ctx, LegacyHistorySchema); e != nil {
			return result, ErrUnavailable
		}
	} else if !records || !receipts {
		return result, ErrConflict
	}
	if tx.QueryRowContext(ctx, `SELECT version FROM lmm_checkins.legacy_history_contract WHERE singleton`).Scan(&version) != nil || version != 1 {
		return result, ErrUnavailable
	}
	if _, e = tx.ExecContext(ctx, `LOCK TABLE lmm_checkins.legacy_history_contract,lmm_checkins.legacy_history_imports,lmm_checkins.legacy_history IN ACCESS EXCLUSIVE MODE`); e != nil {
		return result, ErrUnavailable
	}
	rowJSON, _ := json.Marshal(rows)
	var same bool
	e = tx.QueryRowContext(ctx, `SELECT input_sha256=$2 AND source_namespace_sha256=$3 AND manifest=$4::jsonb
 AND source_rows=$5 AND mapped_rows=$6 AND archived_rows=$7 AND installed_by=session_user
 AND database_oid=(SELECT oid FROM pg_database WHERE datname=current_database()),added_rows
 FROM lmm_checkins.legacy_history_imports WHERE operation_id=$1`, operation, expected, result.SourceNamespaceSHA256, string(raw), result.SourceRows, result.MappedRows, result.ArchivedRows).Scan(&same, &result.AddedRows)
	if e == nil {
		if !same {
			return result, ErrConflict
		}
		var matched int64
		if tx.QueryRowContext(ctx, `WITH incoming AS (`+legacyHistoryRowsSQL+`) SELECT count(*) FROM incoming r JOIN lmm_checkins.legacy_history h ON h.source_namespace_sha256=$2 AND h.source_id=r.source_id WHERE NOT (`+legacyHistoryMismatchSQL+`)`, string(rowJSON), result.SourceNamespaceSHA256).Scan(&matched) != nil || matched != result.SourceRows {
			return result, ErrConflict
		}
		return result, nil
	}
	if !errors.Is(e, sql.ErrNoRows) {
		return result, ErrUnavailable
	}
	if recoverOnly {
		return result, ErrConflict
	}
	// A later full source snapshot may append rows, never drop historical facts,
	// remap a previously committed owner or promote an archived orphan.
	var conflict bool
	e = tx.QueryRowContext(ctx, `WITH incoming AS (`+legacyHistoryRowsSQL+`)
 SELECT EXISTS(SELECT FROM lmm_checkins.legacy_history h LEFT JOIN incoming r USING(source_id)
 WHERE h.source_namespace_sha256=$2 AND (r.source_id IS NULL OR `+legacyHistoryMismatchSQL+`))
 OR EXISTS(SELECT FROM incoming r JOIN lmm_checkins.intentions i ON i.day=r.day
 AND (i.user_id=r.target_user_id OR i.account_id=r.target_personal_account_id))`, string(rowJSON), result.SourceNamespaceSHA256).Scan(&conflict)
	if e != nil {
		return result, ErrUnavailable
	}
	if conflict {
		return result, ErrConflict
	}
	mappingJSON, _ := json.Marshal(input.Mapping)
	e = tx.QueryRowContext(ctx, `WITH incoming AS (SELECT * FROM jsonb_to_recordset($1::jsonb) AS m(source_user_id bigint,target_user_id bigint,target_personal_account_id bigint))
 SELECT EXISTS(SELECT FROM lmm_checkins.legacy_history_imports i
 CROSS JOIN LATERAL jsonb_to_recordset(i.manifest->'mapping') AS old(source_user_id bigint,target_user_id bigint,target_personal_account_id bigint)
 LEFT JOIN incoming m USING(source_user_id)
 WHERE i.source_namespace_sha256=$2 AND (m.source_user_id IS NULL OR m.target_user_id<>old.target_user_id OR m.target_personal_account_id<>old.target_personal_account_id))`, string(mappingJSON), result.SourceNamespaceSHA256).Scan(&conflict)
	if e != nil {
		return result, ErrUnavailable
	}
	if conflict {
		return result, ErrConflict
	}
	if tx.QueryRowContext(ctx, `WITH incoming AS (`+legacyHistoryRowsSQL+`) SELECT count(*) FROM incoming r WHERE NOT EXISTS(SELECT FROM lmm_checkins.legacy_history h WHERE h.source_namespace_sha256=$2 AND h.source_id=r.source_id)`, string(rowJSON), result.SourceNamespaceSHA256).Scan(&result.AddedRows) != nil {
		return result, ErrUnavailable
	}
	if _, e = tx.ExecContext(ctx, `INSERT INTO lmm_checkins.legacy_history_imports(operation_id,input_sha256,source_namespace_sha256,manifest,source_rows,mapped_rows,archived_rows,added_rows,installed_by,database_oid)
 SELECT $1,$2,$3,$4::jsonb,$5,$6,$7,$8,session_user,oid FROM pg_database WHERE datname=current_database()`, operation, expected, result.SourceNamespaceSHA256, string(raw), result.SourceRows, result.MappedRows, result.ArchivedRows, result.AddedRows); e != nil {
		return result, ErrConflict
	}
	if _, e = tx.ExecContext(ctx, `WITH incoming AS (`+legacyHistoryRowsSQL+`) INSERT INTO lmm_checkins.legacy_history
 (source_namespace_sha256,source_id,source_user_id,day,amount_units,created_at,target_user_id,target_personal_account_id,source_row_sha256,import_operation)
 SELECT $2,r.*,$3 FROM incoming r WHERE NOT EXISTS(SELECT FROM lmm_checkins.legacy_history h WHERE h.source_namespace_sha256=$2 AND h.source_id=r.source_id)`, string(rowJSON), result.SourceNamespaceSHA256, operation); e != nil {
		return result, ErrConflict
	}
	role := pq.QuoteIdentifier(runtime)
	if _, e = tx.ExecContext(ctx, `REVOKE ALL ON lmm_checkins.legacy_history_contract,lmm_checkins.legacy_history_imports,lmm_checkins.legacy_history FROM PUBLIC,`+role+`; GRANT SELECT ON lmm_checkins.legacy_history_contract,lmm_checkins.legacy_history TO `+role); e != nil {
		return result, ErrUnavailable
	}
	return result, nil
}

const legacyDailyRecordSQL = `jsonb_build_object('checkin_date',day::text,'quota_awarded',amount_units::text,
 'created_at',created_at,'journal_id',NULL,'origin','history','policy_revision',NULL)`

func loadLegacyHistory(ctx context.Context, tx *sql.Tx, user, account int64, date string) (*DailyRecord, error) {
	rows, e := tx.QueryContext(ctx, `SELECT target_user_id,target_personal_account_id,`+legacyDailyRecordSQL+`
 FROM lmm_checkins.legacy_history WHERE day=$3::date AND (target_user_id=$1 OR target_personal_account_id=$2)`, user, account, date)
	if e != nil {
		return nil, ErrUnavailable
	}
	defer rows.Close()
	var record *DailyRecord
	for rows.Next() {
		var uid, aid int64
		var raw []byte
		var r DailyRecord
		if rows.Scan(&uid, &aid, &raw) != nil || uid != user || aid != account || record != nil || json.Unmarshal(raw, &r) != nil {
			return nil, ErrConflict
		}
		record = &r
	}
	if rows.Err() != nil {
		return nil, ErrUnavailable
	}
	return record, nil
}
