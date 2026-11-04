#!/usr/bin/env python3
# Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
"""Preserve original settlement observations; never authorize or execute payments.

The owner supplies the separately executed pinned Go oracle and frozen source.
Hashes retain provenance, not cryptographic attestation of an arbitrary JSON file.
Independent review, channel binding and runtime settlement remain required.
"""
import argparse
import json
import os
from pathlib import Path
import re

from legacy_cutover_money import topup_can_settle
from owner_history_receipt import owner_history_import_sql
from prepare_wallet_history_snapshot import canonical, digest, require

SCOPE='legacy-topup-settlement-facts.v1'
NAMESPACE='lmm_api.lmm_prod_20260802.unsettled_topup_basis'
LEGACY_COMMIT='3948a0a1ee1cefdd016a7aa45a11851545bb4b85'
MAX=9007199254740991


def prepare(source, oracle, *, archive_sha256, expected_rows):
    require(re.fullmatch('[0-9a-f]{64}',archive_sha256) is not None,'archive_hash_invalid')
    require(source.get('source_namespace')==NAMESPACE,'source_namespace_mismatch')
    observed=source.get('observed_at')
    require(type(observed) is int and 0<observed<=MAX,'observation_invalid')
    users=source.get('source_user_ids');tables=source.get('tables')
    require(type(users) is list and users and all(type(u) is int and 0<u<=MAX for u in users)
            and len(users)==len(set(users)),'source_users_invalid')
    require(type(tables) is dict and set(tables)=={'top_ups','wallet_credit_rebases','credit_options'}
            and all(type(v) is list for v in tables.values()),'source_tables_invalid')
    rows=tables['top_ups']
    require(type(expected_rows) is int and expected_rows>=0 and len(rows)==expected_rows,'source_count_mismatch')
    require(oracle.get('source_commit')==LEGACY_COMMIT and oracle.get('source_orders_sha256')==digest(rows),
            'oracle_source_binding_mismatch')
    require(oracle.get('source_tables_sha256')==digest(tables) and oracle.get('source_user_ids_sha256')==digest(users),
            'oracle_source_context_mismatch')
    require(oracle.get('network_isolated') is True and oracle.get('transaction_rolled_back') is True,
            'oracle_execution_scope_invalid')
    for key in ('binary_sha256','capture_source_sha256','original_archive_sha256'):
        require(type(oracle.get(key)) is str and re.fullmatch('[0-9a-f]{64}',oracle[key]) is not None,'oracle_fingerprint_missing')
    results=oracle.get('results')
    require(type(results) is list and len(results)==len(rows),'oracle_coverage_mismatch')
    by_id={}
    for r in results:
        require(type(r) is dict and set(r)=={'id','user_id','quota','expected_micros','rejected'},'oracle_record_shape_invalid')
        require(type(r['id']) is int and 0<r['id']<=MAX and r['id'] not in by_id,'oracle_duplicate_or_invalid_id')
        require(type(r['user_id']) is int and 0<r['user_id']<=MAX,'oracle_owner_invalid')
        require(type(r['rejected']) is bool and type(r['quota']) is int and
                ((r['rejected'] and r['quota']==0) or(not r['rejected'] and 0<r['quota']<=MAX)),'oracle_quota_invalid')
        require(type(r['expected_micros']) is int and 0<r['expected_micros']<=2**63-1,'oracle_amount_unavailable')
        by_id[r['id']]=r
    seen=set();trades=set()
    for row in rows:
        require(type(row) is dict and type(row.get('id')) is int and 0<row['id']<=MAX and row['id'] not in seen,'source_duplicate_or_invalid_id')
        seen.add(row['id'])
        require(type(row.get('user_id')) is int and 0<row['user_id']<=MAX and topup_can_settle(row),'source_not_unsettled')
        require(type(row.get('trade_no')) is str and row['trade_no'] and row['trade_no'] not in trades,'source_trade_reference_invalid')
        trades.add(row['trade_no'])
        require(type(row.get('money')) is str and re.fullmatch(r'[0-9]+(?:\.[0-9]+)?',row['money']) is not None,'source_money_not_exact_text')
        require(row['id'] in by_id and by_id[row['id']]['user_id']==row['user_id'],'oracle_owner_or_order_mismatch')
    require(seen==set(by_id),'oracle_coverage_mismatch')
    known=set(users)
    return {'scope':SCOPE,'source_namespace':NAMESPACE,'migration_id':'legacy-topup-settlement-facts-v1',
            'source_archive_sha256':archive_sha256,'observed_at':observed,'source_user_ids':sorted(users),
            'mapping_sha256':digest(sorted(users)),'source_export_sha256':digest(source),
            'source_order_count':len(rows),'source_orders_sha256':digest(rows),'history_complete':True,
            'balance_effect':'none','runtime_settlement_available':False,
            'quota_unit':'original_runtime_effective_quota_not_new_pricing',
            'source_money_policy':'numeric_money_as_exact_text',
            'original_quota_accepted_count':sum(not r['rejected'] for r in results),
            'original_quota_rejected_count':sum(r['rejected'] for r in results),
            'unmapped_order_ids':sorted(r['id'] for r in rows if r['user_id'] not in known),
            'channel_binding':'requires_separate_owner_review',
            'oracle':oracle,'tables':tables}


def import_sql(manifest):
    return owner_history_import_sql(manifest,scope=SCOPE,error_prefix='legacy_settlement_facts')


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--input',type=Path,required=True);p.add_argument('--oracle',type=Path,required=True)
    p.add_argument('--output',type=Path,required=True);p.add_argument('--archive-sha256',required=True)
    p.add_argument('--expected-rows',type=int,required=True);a=p.parse_args()
    m=prepare(json.loads(a.input.read_text()),json.loads(a.oracle.read_text()),archive_sha256=a.archive_sha256,expected_rows=a.expected_rows)
    a.output.mkdir(mode=0o700,parents=True,exist_ok=True)
    for name,text in [('manifest.private.json',canonical(m)+'\n'),('import.private.sql',import_sql(m))]:
        with os.fdopen(os.open(a.output/name,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600),'w') as f:f.write(text)
    print(json.dumps({k:m[k] for k in ('source_order_count','original_quota_accepted_count','original_quota_rejected_count','balance_effect','runtime_settlement_available')}))


if __name__=='__main__':main()
