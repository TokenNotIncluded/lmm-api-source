#!/usr/bin/env python3
# Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
"""Prepare private owner-reviewed frozen bases. Never connect or post money."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re

from prepare_legacy_settlement_basis import prepare as settlement_facts
from prepare_wallet_history_snapshot import canonical, digest, literal, require

SCOPE = 'owner-reviewed-legacy-payment-bindings.v2'
INTS = {'source_order_id', 'source_user_id', 'user_id', 'account_id', 'channel_id',
        'amount_micros', 'credit_units', 'discount_code_id'}
HASHES = {'source_row_sha256', 'review_sha256'}
BINDING_FIELDS = {'source_order_id', 'source_row_sha256', 'user_id', 'account_id',
                  'channel_id', 'merchant', 'environment', 'currency', 'store_id', 'source_product_id', 'opening_entity_key'}


def candidates(facts):
    known = set(facts['source_user_ids'])
    quotes = {r['id']: r for r in facts['oracle']['results']}
    eligible, gaps = [], []
    for row in facts['tables']['top_ups']:
        reason = ('unmapped_owner' if row['user_id'] not in known else
                  'original_quota_rejected' if quotes[row['id']]['rejected'] else
                  'unsupported_provider' if row.get('payment_provider') != 'waffo_pancake' else None)
        if reason:
            gaps.append({'source_order_id': row['id'], 'reason': reason})
        else:
            eligible.append(row)
    return eligible, gaps


def draft(facts):
    eligible, gaps = candidates(facts)
    rows = []
    for r in eligible:
        rows.append(dict(source_order_id=r['id'], source_row_sha256=digest(r),
                         user_id=None, account_id=None, opening_entity_key=None, channel_id=None, merchant=None,
                         environment=None, currency=(r.get('settlement_currency') or '').strip().upper() or None,
                         store_id=r.get('provider_store_id') or None,
                         source_product_id=r.get('provider_product_id') or None))
    return dict(scope='draft-legacy-payment-bindings.v1', facts_sha256=digest(facts),
                opening_migration_id=None, orders=rows, gaps=gaps,
                installable=False, balance_effect='none')


def prepare(facts, bindings):
    require(type(bindings) is dict and set(bindings) == {'scope', 'facts_sha256', 'opening_migration_id', 'orders'}, 'binding_document_invalid')
    require(bindings['scope'] == SCOPE and bindings['facts_sha256'] == digest(facts), 'binding_facts_mismatch')
    migration = bindings['opening_migration_id']
    require(type(migration) is str and re.fullmatch(r'[A-Za-z0-9._:/-]{1,128}', migration), 'opening_migration_invalid')
    require(type(bindings['orders']) is list and bindings['orders'], 'binding_orders_missing')
    eligible, gaps = candidates(facts)
    by_id = {r['id']: r for r in eligible}
    quotes = {r['id']: r for r in facts['oracle']['results']}
    bases, seen = [], set()
    for binding in bindings['orders']:
        require(type(binding) is dict and set(binding) == BINDING_FIELDS, 'binding_row_shape_invalid')
        oid = binding['source_order_id']
        require(type(oid) is int and oid in by_id and oid not in seen, 'binding_order_ineligible_or_duplicate')
        seen.add(oid)
        row = by_id[oid]
        require(binding['source_row_sha256'] == digest(row), 'binding_source_row_mismatch')
        require(binding['opening_entity_key'] in (f"user:{row['user_id']}", f"legacy.users/{row['user_id']}"), 'binding_opening_identity_invalid')
        for key in ('user_id', 'account_id', 'channel_id'):
            require(type(binding[key]) is int and 0 < binding[key] <= 2**63-1, 'binding_identity_invalid')
        for key in ('merchant', 'store_id'):
            require(type(binding[key]) is str and 1 <= len(binding[key]) <= 128, 'binding_channel_missing')
        require(binding['environment'] in ('test', 'live'), 'binding_environment_invalid')
        require(type(binding['currency']) is str and re.fullmatch('[A-Z]{3}', binding['currency']), 'binding_currency_missing')
        require(re.fullmatch('[A-Za-z0-9_.-]{1,128}', row['trade_no']), 'binding_reference_unsupported')
        original_currency = (row.get('settlement_currency') or '').strip().upper()
        require(not original_currency or original_currency == binding['currency'], 'binding_original_currency_changed')
        require(not row.get('provider_store_id') or row['provider_store_id'] == binding['store_id'], 'binding_original_store_changed')
        require((row.get('provider_product_id') or None) == binding['source_product_id'], 'binding_original_product_changed')
        product = binding['source_product_id']
        require(product is None or type(product) is str and 1 <= len(product) <= 128, 'binding_product_invalid')
        # Original Go scans this nullable column into an int, where NULL is zero.
        discount = 0 if row.get('discount_code_id') is None else row['discount_code_id']
        require(type(discount) is int and 0 <= discount <= 2**63-1, 'binding_discount_invalid')
        basis = dict(binding)
        basis.update(id='legacy_' + digest([facts['source_archive_sha256'], oid]),
                     opening_migration_id=migration, source_archive_sha256=facts['source_archive_sha256'],
                     source_user_id=row['user_id'], provider='waffo_pancake', order_reference=row['trade_no'],
                     amount_micros=quotes[oid]['expected_micros'], credit_units=quotes[oid]['quota'],
                     discount_code_id=discount, source_status=row['status'],
                     failure_reason=row.get('failure_reason_code') or '',
                     review_sha256=digest({'scope': SCOPE, 'facts_sha256': bindings['facts_sha256'],
                                           'opening_migration_id': migration, 'binding': binding}))
        bases.append(basis)
    return dict(scope=SCOPE, facts_sha256=digest(facts), facts=facts,
                bases=sorted(bases, key=lambda r: r['source_order_id']),
                excluded=gaps, unselected_order_ids=sorted(set(by_id)-seen),
                balance_effect='none', performs_network_io=False)


def import_sql(plan):
    payload = literal(plan)
    tag = '$legacy_' + digest(plan) + '$'
    require(tag not in payload, 'binding_sql_delimiter_collision')
    columns = sorted(plan['bases'][0])
    values = [f"decode(r->>'{k}','hex')" if k in HASHES else
              f"(r->>'{k}')::bigint" if k in INTS else f"r->>'{k}'" for k in columns]
    return f"""BEGIN ISOLATION LEVEL SERIALIZABLE;
SET LOCAL standard_conforming_strings=on;
SET LOCAL synchronous_commit=on;
SET LOCAL lock_timeout='5s';
SET LOCAL statement_timeout='60s';
SELECT pg_advisory_xact_lock(741803122);
DO {tag}
DECLARE p jsonb := {payload}::jsonb; r jsonb; prior core_payments.legacy_bases%ROWTYPE;
BEGIN
 IF NOT EXISTS(SELECT FROM pg_class WHERE oid='core_payments.legacy_bases'::regclass
   AND pg_get_userbyid(relowner)=current_user)
 THEN RAISE EXCEPTION 'legacy_binding_installation_owner_required'; END IF;
 IF NOT EXISTS(SELECT FROM core_migration.import_runs
   WHERE migration_id=p->'facts'->>'migration_id' AND manifest=p->'facts')
 THEN RAISE EXCEPTION 'legacy_binding_installed_facts_mismatch'; END IF;
 IF NOT EXISTS(SELECT FROM core_billing.opening_import_runs
   WHERE migration_id=p->'bases'->0->>'opening_migration_id'
     AND source_archive_sha256=p->'facts'->>'source_archive_sha256'
     AND observed_at=(p->'facts'->>'observed_at')::bigint)
 THEN RAISE EXCEPTION 'legacy_binding_opening_mismatch'; END IF;
 FOR r IN SELECT value FROM jsonb_array_elements(p->'bases') LOOP
  SELECT * INTO prior FROM core_payments.legacy_bases WHERE id=r->>'id';
  IF FOUND THEN
   IF (to_jsonb(prior)-ARRAY['source_row_sha256','review_sha256'])
      IS DISTINCT FROM (r-ARRAY['source_row_sha256','review_sha256'])
      OR prior.source_row_sha256 IS DISTINCT FROM decode(r->>'source_row_sha256','hex')
      OR prior.review_sha256 IS DISTINCT FROM decode(r->>'review_sha256','hex')
   THEN RAISE EXCEPTION 'legacy_binding_replay_conflict'; END IF;
  ELSE
   INSERT INTO core_payments.legacy_bases({','.join(columns)})
   VALUES({','.join(values)});
  END IF;
 END LOOP;
END {tag};
COMMIT;
"""


def main():
    p = argparse.ArgumentParser(description=__doc__)
    for key in ('input', 'oracle', 'output'):
        p.add_argument('--'+key, type=Path, required=True)
    p.add_argument('--bindings', type=Path)
    p.add_argument('--archive-sha256', required=True)
    p.add_argument('--expected-rows', type=int, required=True)
    a = p.parse_args()
    facts = settlement_facts(json.loads(a.input.read_text()), json.loads(a.oracle.read_text()),
                             archive_sha256=a.archive_sha256, expected_rows=a.expected_rows,
                             source_input_sha256=hashlib.sha256(a.input.read_bytes()).hexdigest())
    if a.bindings:
        plan = prepare(facts, json.loads(a.bindings.read_text()))
        files = [('plan.private.json', canonical(plan)+'\n'), ('install.private.sql', import_sql(plan))]
    else:
        plan = draft(facts)
        files = [('bindings.template.private.json', canonical(plan)+'\n')]
    a.output.mkdir(mode=0o700, parents=True, exist_ok=True)
    require(a.output.stat().st_mode & 0o077 == 0, 'output_directory_not_private')
    for name, text in files:
        with os.fdopen(os.open(a.output/name, os.O_WRONLY|os.O_CREAT|os.O_EXCL, 0o600), 'w') as f:
            f.write(text)
    print(json.dumps({'selected_orders': len(plan.get('bases', [])),
                      'draft_orders': len(plan.get('orders', [])), 'balance_effect': 'none'}))


if __name__ == '__main__':
    main()
