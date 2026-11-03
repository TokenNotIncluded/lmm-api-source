#!/usr/bin/env python3
# Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
"""Read-only settlement-evidence audit for legacy acquisition cash metrics.

Internal credits are excluded. A subscription order's expected amount is a
quote, not proof of receipt. Parent orders and top-up mirrors are deduplicated;
immutable subscription cycle events take precedence over their parent order.
This checks field presence, not provider receipt authenticity or full migration
acceptance. Only aggregate counts leave the local restored source database.
"""
import argparse
import json
import os
from pathlib import Path
import re
from legacy_acquisition_rehearsal import psql


def audit_sql(schema):
    if not re.fullmatch(r'[a-z_][a-z_0-9]*', schema):
        raise ValueError('invalid legacy schema')
    return f"""BEGIN READ ONLY;
WITH events AS (
 SELECT 'topup'::text AS kind,t.user_id,t.complete_time AS at,t.settled_amount_micros AS amount,t.settlement_currency AS currency,t.payment_method AS method,t.payment_provider AS provider,'settled'::text AS basis
 FROM {schema}.top_ups t WHERE t.status='success' AND NOT EXISTS(SELECT 1 FROM {schema}.subscription_orders s WHERE s.trade_no=t.trade_no AND s.status='success')
 UNION ALL SELECT 'subscription_order',s.user_id,s.complete_time,s.expected_amount_micros,s.settlement_currency,s.payment_method,s.payment_provider,'quoted_only'
 FROM {schema}.subscription_orders s WHERE s.status='success' AND NOT EXISTS(SELECT 1 FROM {schema}.subscription_payment_events e WHERE e.subscription_order_id=s.id)
 UNION ALL SELECT 'subscription_cycle',s.user_id,e.created_time,e.settlement_amount_micros,e.settlement_currency,s.payment_method,e.payment_provider,'settled'
 FROM {schema}.subscription_payment_events e JOIN {schema}.subscription_orders s ON s.id=e.subscription_order_id WHERE s.status='success'
 UNION ALL SELECT 'refund',f.user_id,f.occurred_at,f.amount_micros,f.currency,f.payment_method,f.payment_provider,
 CASE WHEN f.direction IN(-1,1) THEN 'refund' ELSE 'invalid_refund_direction' END
 FROM {schema}.finance_ledger_entries f WHERE f.entry_type='revenue' AND f.source_type='refund'
), normalized AS (
 SELECT *,lower(trim(coalesce(method,''))) AS m,lower(trim(coalesce(provider,''))) AS p,upper(trim(coalesce(currency,''))) AS c FROM events
), classified AS (
 SELECT kind,CASE
 WHEN m='balance' OR p='balance' OR m IN('gift','bonus','checkin','invite','bounty','linuxdo','linux_do','linuxdo_credit','internal','admin') OR p IN('gift','bonus','checkin','invite','bounty','linuxdo','linux_do','linuxdo_credit','internal','admin') OR (p='epay' AND m IN('ldc','linuxdo','linux_do','linuxdo_credit')) THEN 'internal_credit'
 WHEN m='' AND p='' THEN 'missing_payment_source'
 WHEN basis='quoted_only' THEN 'subscription_cash_unproven'
 WHEN basis='invalid_refund_direction' THEN basis
 WHEN user_id IS NULL OR user_id<=0 THEN 'missing_account'
 WHEN at IS NULL OR at<=0 THEN 'missing_settlement_time'
 WHEN amount IS NULL OR amount<=0 THEN 'missing_settled_amount'
 WHEN c !~ '^[A-Z]{{3}}$' THEN 'missing_settlement_currency'
 ELSE 'cash_evidence_present' END AS evidence FROM normalized
), groups AS (SELECT kind,evidence,count(*) AS rows FROM classified GROUP BY kind,evidence ORDER BY kind,evidence)
SELECT json_build_object('contract','legacy-acquisition-cash-evidence.v1',
 'scope','field_presence_only','financial_acceptance_complete',false,
 'event_rows',(SELECT count(*) FROM classified),
 'cash_evidence_rows',(SELECT count(*) FROM classified WHERE evidence='cash_evidence_present'),
 'internal_credit_rows',(SELECT count(*) FROM classified WHERE evidence='internal_credit'),
 'unproven_rows',(SELECT count(*) FROM classified WHERE evidence NOT IN('cash_evidence_present','internal_credit')),
 'groups',coalesce((SELECT json_agg(groups) FROM groups),'[]'::json));
ROLLBACK;
"""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--socket', required=True, type=Path)
    parser.add_argument('--port', required=True, type=int)
    parser.add_argument('--source', default='legacy')
    parser.add_argument('--schema', required=True)
    parser.add_argument('--report', required=True, type=Path)
    args = parser.parse_args()
    backup = (Path.home() / 'Backups').resolve()
    socket, report = args.socket.resolve(), args.report.resolve()
    if not socket.is_relative_to(backup) or not socket.is_dir() or not report.is_relative_to(backup) or report.exists() or not 1 <= args.port <= 65535 or not re.fullmatch(r'[a-z_][a-z_0-9]*', args.source):
        parser.error('requires a local restored database and new private report under Backups')
    result = json.loads(psql(socket, args.port, args.source, audit_sql(args.schema), True))
    with os.fdopen(os.open(report, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), 'w') as stream:
        json.dump(result, stream, sort_keys=True)
        stream.write('\n')
    print(json.dumps(result, sort_keys=True))
    raise SystemExit(2 if result['unproven_rows'] else 0)


if __name__ == '__main__':
    main()
