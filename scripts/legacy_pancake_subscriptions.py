# Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
"""Owner-reviewed original subscription bindings; offline SQL, no provider calls.

This writer never creates wallet money or charging grants. Published immutable
Core revisions and independently imported legacy entitlements must already exist.
A financial target attestation supplies identity, catalog, ACL and offline fences;
it is not approval of the source terms conversion. The separate review binds it.
"""
from __future__ import annotations
import copy
import json
import re
from pathlib import Path
import financial_import as F

SCOPE = 'legacy-pancake-subscription-originals-v1'
CHANGED = ['core_subscriptions.external_prices', 'core_subscriptions.external_orders',
           'core_subscriptions.pancake_prices', 'core_subscriptions.pancake_orders',
           'core_subscriptions.commands']
require = F.require
integer = F.PLANNER.integer

def identifier(v):
    require(type(v) is str and re.fullmatch(r'[A-Za-z0-9_.-]{1,128}', v), 'original_identifier_invalid')
    return v

def build_records(source, review, native_users):
    require(type(source) is dict and type(source.get('orders')) is list, 'original_source_invalid')
    archive = F.checked_hash(source.get('source_archive_sha256'))
    require(type(review) is dict and review.get('source_archive_sha256') == archive
            and review.get('decision') == 'approve-original-subscription-bindings', 'original_review_required')
    require(type(review.get('orders')) is list, 'original_review_shape')
    bindings = {integer(b.get('source_order_id'), 'original_id_invalid', minimum=1): b for b in review['orders']}
    require(len(bindings) == len(review['orders']), 'duplicate_original_review')
    users = {r['id']: r['personal_account_id'] for r in native_users}
    result, seen = [], set()
    for row in source['orders']:
        require(type(row) is dict, 'original_row_invalid')
        provider = row.get('payment_provider')
        if provider not in ('waffo_pancake', 'balance', 'epay'):
            continue  # Other providers retain their own unresolved liabilities.
        oid = integer(row.get('id'), 'original_id_invalid', minimum=1)
        require(oid not in seen and oid in bindings, 'original_missing_or_duplicate_review'); seen.add(oid)
        b = bindings[oid]
        require(b.get('source_row_sha256') == F.digest(row), 'original_source_row_changed')
        user = integer(row.get('user_id'), 'original_user_invalid', minimum=1)
        require(user in users and b.get('account_id') == users[user], 'original_identity_mismatch')
        reference = identifier(row.get('trade_no'))
        status = row.get('status')
        require(status in ('pending', 'success') and (provider != 'balance' or status == 'success'), 'original_status_unhandled')
        require(not any(row.get(k, 0) for k in ('refunded_amount_micros', 'refunded_quota')), 'original_refund_unreviewed')
        created = row.get('create_time',row.get('created_at'))
        integer(created, 'original_creation_invalid', minimum=1)
        if row.get('create_time') is not None and row.get('created_at') is not None:
            require(row['create_time']==row['created_at'], 'original_creation_ambiguous')
        gaps=[]
        field=b.get('snapshot_field')
        snapshot=row.get(field) if type(field) is str else None
        if snapshot in (None,''): gaps.append('original_snapshot_missing')
        else:
            try:
                parsed=json.loads(snapshot) if type(snapshot) is str else snapshot
                if type(parsed) is not dict:gaps.append('original_snapshot_invalid')
            except (ValueError,TypeError):gaps.append('original_snapshot_invalid')
        if provider!='balance':
            cash=row.get('expected_amount_micros')
            if type(cash) is not int or cash<=0:gaps.append('original_cash_missing')
            elif cash%10000!=0:gaps.append('original_cash_not_exact_minor')
            if row.get('settlement_currency') not in ('USD','CNY','LDC'):gaps.append('original_currency_missing')
            if provider=='waffo_pancake' and (not row.get('provider_product_id') or not row.get('provider_store_id')):
                gaps.append('original_product_store_missing')
        elif type(row.get('charged_quota')) is not int or row['charged_quota']<=0:
            gaps.append('original_balance_charge_missing')
        if gaps:
            require(b.get('disposition')=='retain-unknown', 'original_missing_material_requires_explicit_unknown')
            result.append({'source_order_id':oid,'actor_user_id':user,'account_id':users[user],
                'provider':provider,'reference':reference,'disposition':'UNKNOWN',
                'operation_key':'legacy_unknown_'+F.digest([archive,oid]),
                'unknown':{'state':'UNKNOWN','source_order_id':str(oid),'account_id':str(users[user]),
                    'provider':provider,'reference':reference,'source_status':status,'created_at':str(created),
                    'source_archive_sha256':archive,'source_row_sha256':F.digest(row),'missing_material':gaps}})
            continue
        require(b.get('disposition')!='retain-unknown', 'original_complete_material_cannot_be_discarded')
        require(provider!='epay', 'complete_legacy_epay_requires_its_own_binding_writer')
        plan = b.get('plan'); require(type(plan) is dict and set(plan) == {'id', 'revision', 'archived', 'terms'}, 'original_core_plan_invalid')
        for key in ('id', 'revision'): integer(plan[key], 'original_plan_id_invalid', minimum=1)
        require(type(plan['terms']) is dict and type(plan['archived']) is bool, 'original_terms_invalid')
        field = b.get('snapshot_field')
        require(type(field) is str and field in row and row[field] not in ('', None), 'original_frozen_snapshot_missing')
        snapshot = row[field]
        if type(snapshot) is str:
            snapshot = json.loads(snapshot)
        require(type(snapshot) is dict, 'original_frozen_snapshot_invalid')
        terms_review = b.get('terms_review', {})
        require(terms_review == {'decision': 'equivalent-no-unsupported-policy',
            'source_snapshot_sha256': F.digest(snapshot), 'core_terms_sha256': F.digest(plan['terms'])}, 'original_terms_equivalence_unreviewed')
        require(not any(snapshot.get(k) for k in ('upgrade_group', 'downgrade_group', 'previous_group', 'allow_wallet_overflow')), 'original_policy_unrepresentable')
        legacy = {'source_archive_sha256': archive, 'source_order_id': oid, 'source_row_sha256': F.digest(row),
                  'source_snapshot_sha256': F.digest(snapshot), 'source_status': status}
        grant_ref = b.get('opening_grant_reference')
        cutoff = b.get('last_paid_period_end', 0)
        if status == 'success':
            require(type(grant_ref) is str and grant_ref.startswith('legacy-sub:') and len(grant_ref) <= 200, 'original_opening_grant_required')
            cutoff = integer(cutoff, 'original_paid_cutoff_required', minimum=1)
            legacy.update(opening_grant_reference=grant_ref, last_paid_period_end=cutoff)
        else:
            require(cutoff == 0 and grant_ref is None, 'original_pending_cannot_claim_paid_period')
        record = {'source_order_id': oid, 'actor_user_id': user, 'account_id': users[user],
                  'provider': provider, 'reference': reference, 'plan': plan, 'legacy_source': legacy,
                  'quoted_at': created, 'cutoff': cutoff, 'opening_grant_reference': grant_ref}
        if provider == 'balance':
            # Old balance purchases are immutable history; their debit is already
            # conserved by opening, and their grant by the entitlement importer.
            charge = row.get('charged_quota')
            integer(charge, 'original_balance_charge_missing', minimum=1)
            charge_units = b.get('charge_units')
            require(type(charge_units) is str and charge_units.isascii() and charge_units.isdigit()
                    and str(int(charge_units)) == charge_units and int(charge_units)>0, 'original_balance_units_invalid')
            unit_hash = F.checked_hash(review.get('unit_mapping_sha256'))
            require(b.get('charge_unit_review') == {'decision':'approved-source-quota-to-core-units',
                'source_charged_quota':charge, 'core_charge_units':charge_units,
                'unit_mapping_sha256':unit_hash}, 'original_balance_units_unreviewed')
            record.update(operation_key='legacy_balance_' + F.digest([archive, oid]), charge_units=charge_units)
        else:
            require(reference.startswith('WAFFO_PANCAKE_SUB-'), 'original_subscription_reference_invalid')
            amount = integer(row.get('expected_amount_micros'), 'original_exact_cash_missing', minimum=1)
            currency = row.get('settlement_currency')
            require(currency in ('USD', 'CNY') and amount % 10000 == 0, 'original_cash_not_exact_minor')
            p = b.get('binding'); require(type(p) is dict and set(p) == {'merchant', 'environment', 'price'}, 'original_binding_invalid')
            price = p['price']; require(type(price) is dict and set(price) == {
                'plan_id','plan_revision','channel_id','currency','amount_minor','store_id','product_id',
                'product_type','billing_period','provider_plan_reference','return_url','checkout_region',
                'checkout_language','tax_category'}, 'original_price_invalid')
            channel = price['channel_id']
            require(type(channel) in (int,str) and str(channel).isascii() and str(channel).isdigit() and str(int(channel))==str(channel), 'original_channel_invalid')
            integer(int(channel), 'original_channel_invalid', minimum=1)
            require(p['environment'] in ('live','test') and type(p['merchant']) is str and p['merchant'], 'original_namespace_invalid')
            require(str(price.get('plan_id')) == str(plan['id']) and str(price.get('plan_revision')) == str(plan['revision']), 'original_price_plan_mismatch')
            require(price.get('currency') == currency and price.get('amount_minor') == str(amount // 10000), 'original_cash_binding_mismatch')
            require(price.get('product_id') == row.get('provider_product_id') and price.get('store_id') == row.get('provider_store_id'), 'original_product_binding_mismatch')
            require(price.get('provider_plan_reference') == str(row.get('plan_id')), 'original_provider_plan_mismatch')
            require(price.get('product_type') in ('one_time', 'subscription'), 'original_product_type_invalid')
            period = price.get('billing_period')
            require((price['product_type']=='one_time' and period=='') or
                    (price['product_type']=='subscription' and currency=='USD' and period in ('weekly','monthly','quarterly','yearly')),
                    'original_billing_period_unrepresentable')
            original_provider_order = row.get('provider_subscription_id') or None
            require(b.get('provider_order_id') == original_provider_order, 'original_provider_order_mismatch')
            if original_provider_order: identifier(original_provider_order)
            record.update(order_id='sub_' + F.digest([SCOPE, archive, oid]), operation_key='legacy_pancake_' + F.digest([archive, oid]),
                binding=copy.deepcopy(p), provider_order_id=original_provider_order)
        result.append(record)
    require(seen == set(bindings), 'original_review_unknown_order')
    require(len({r['reference'] for r in result}) == len(result), 'duplicate_original_reference')
    prices = {}
    for r in result:
        if r['provider']!='waffo_pancake' or r.get('disposition')=='UNKNOWN': continue
        p = r['binding']['price']; key = (r['plan']['id'],r['plan']['revision'],str(p['channel_id']))
        require(key not in prices or prices[key]==p, 'original_price_revision_conflict')
        prices[key]=p
    return sorted(result, key=lambda r: r['source_order_id'])


def prepare_material(attestation, source, review, *, migration_id):
    F.validate_material(attestation)
    require(type(migration_id) is str and F.IDENTITY.MIGRATION_ID.fullmatch(migration_id), 'original_migration_id_invalid')
    require(source.get('source_archive_sha256') == attestation['source_archive_sha256'], 'original_archive_mismatch')
    records = build_records(source, review, attestation['native_users'])
    if any(r['provider']=='balance' and r.get('disposition')!='UNKNOWN' for r in records):
        require(review['unit_mapping_sha256']==F.digest(attestation['source_review']), 'original_financial_unit_review_mismatch')
    return {'format_version': 1, 'scope': SCOPE, 'migration_id': migration_id,
            'financial_attestation': copy.deepcopy(attestation), 'source': copy.deepcopy(source), 'review': copy.deepcopy(review),
            'records': records, 'mapping_sha256': F.digest(records), 'writer_sha256': F.sha(Path(__file__).read_bytes())}


def validate_material(m):
    require(m.get('format_version') == 1 and m.get('scope') == SCOPE, 'original_material_scope')
    require(m == prepare_material(m['financial_attestation'], m['source'], m['review'], migration_id=m['migration_id']), 'original_material_changed')


def build_apply_sql(m):
    validate_material(m)
    e = {**m['financial_attestation'], 'migration_id': m['migration_id']}
    inv = e['before_state']['inventory']; state = F.state_sql(inv, m['migration_id']); guard = F._guard(e)
    changed = 'ARRAY[' + ','.join(F.literal(t) for t in CHANGED) + ']::text[]'
    sql = F._start(e, readonly=False) + f'''
DO $lmm_originals$
DECLARE m jsonb:={F.literal(m)}::jsonb; p jsonb; b jsonb; t jsonb; price jsonb; before_state jsonb; after_state jsonb;
 previous core_migration.import_runs%ROWTYPE; grant_id bigint; original_order text; original_channel bigint; original_plan bigint; original_revision bigint;
BEGIN
 {guard}
 LOCK TABLE {','.join(F.relation(t) for t in inv['tables'])} IN ACCESS EXCLUSIVE MODE;
 {guard}
 SELECT ({state}) INTO before_state;
 SELECT * INTO previous FROM core_migration.import_runs WHERE migration_id=m->>'migration_id';
 IF FOUND THEN
  IF previous.source_archive_sha256 IS DISTINCT FROM m->'financial_attestation'->>'source_archive_sha256'
   OR previous.mapping_sha256 IS DISTINCT FROM m->>'mapping_sha256'
   OR previous.manifest->'material' IS DISTINCT FROM m OR previous.manifest->>'scope' IS DISTINCT FROM {F.literal(SCOPE)}
   OR previous.manifest->'post_state' IS DISTINCT FROM before_state THEN RAISE EXCEPTION 'original_replay_conflict'; END IF;
 ELSE
  IF before_state IS DISTINCT FROM m->'financial_attestation'->'before_state'
   THEN RAISE EXCEPTION 'original_target_changed'; END IF;
  IF NOT EXISTS(SELECT FROM core_migration.import_runs r WHERE to_jsonb(r)=m->'financial_attestation'->'identity_import')
   OR (SELECT coalesce(jsonb_agg(jsonb_build_object('id',id,'personal_account_id',personal_account_id) ORDER BY id),'[]'::jsonb)
       FROM core_identity.users) IS DISTINCT FROM m->'financial_attestation'->'native_users'
   THEN RAISE EXCEPTION 'original_identity_changed'; END IF;
  FOR p IN SELECT value FROM jsonb_array_elements(m->'records') LOOP
   IF p->>'disposition'='UNKNOWN' THEN
    INSERT INTO core_subscriptions.commands(actor_user_id,operation_key,request,result)
    VALUES((p->>'actor_user_id')::bigint,p->>'operation_key',jsonb_build_object('kind','legacy_subscription_unknown_v1'),p->'unknown');
   ELSE
   original_plan:=(p->'plan'->>'id')::bigint; original_revision:=(p->'plan'->>'revision')::bigint;
   IF NOT EXISTS(SELECT FROM core_subscriptions.plan_revisions pr WHERE pr.plan_id=original_plan AND pr.revision=original_revision
        AND terms=p->'plan'->'terms') THEN RAISE EXCEPTION 'original_plan_revision_missing_or_changed'; END IF;
   grant_id:=NULL;
   IF p->'legacy_source'->>'source_status'='success' THEN
    SELECT id INTO STRICT grant_id FROM core_charging.subscriptions WHERE grant_reference=p->>'opening_grant_reference'
     AND account_id=(p->>'account_id')::bigint AND ends_at=(p->>'cutoff')::bigint;
    IF NOT EXISTS(SELECT FROM core_migration.import_runs r,
        LATERAL jsonb_array_elements(r.manifest->'material'->'records') opening
      WHERE r.manifest->>'scope'='subscription-opening-only-v1'
       AND r.source_archive_sha256=m->'financial_attestation'->>'source_archive_sha256'
       AND 'legacy-sub:'||r.migration_id||':'||(opening->>'source_subscription_id')=p->>'opening_grant_reference'
       AND (opening->>'source_user_id')::bigint=(p->>'actor_user_id')::bigint
       AND (opening->>'account_id')::bigint=(p->>'account_id')::bigint
       AND (opening->>'ends_at')::bigint=(p->>'cutoff')::bigint)
     THEN RAISE EXCEPTION 'original_opening_receipt_mismatch'; END IF;
   END IF;
   IF p->>'provider'='balance' THEN
    IF EXISTS(SELECT FROM core_charging.subscriptions WHERE id=grant_id AND (active OR ends_at>extract(epoch FROM clock_timestamp())))
     THEN RAISE EXCEPTION 'active_legacy_balance_lifecycle_not_mapped'; END IF;
    INSERT INTO core_subscriptions.commands(actor_user_id,operation_key,request,result)
    VALUES((p->>'actor_user_id')::bigint,p->>'operation_key',jsonb_build_object('kind','legacy_balance_original_v1'),
      jsonb_build_object('reference',p->>'reference','plan',p->'plan','account_id',p->'account_id',
       'charge_units',p->>'charge_units','subscription_id',grant_id,'ends_at',p->'cutoff','source',p->'legacy_source'));
   ELSE
    b:=p->'binding';price:=b->'price'; original_channel:=(price->>'channel_id')::bigint;original_order:=p->>'order_id';
    INSERT INTO core_subscriptions.external_prices(plan_id,plan_revision,channel_id,currency,amount_minor,actor_user_id)
    VALUES(original_plan,original_revision,original_channel,price->>'currency',(price->>'amount_minor')::bigint,(p->>'actor_user_id')::bigint)
    ON CONFLICT DO NOTHING;
    IF NOT EXISTS(SELECT FROM core_subscriptions.external_prices ep WHERE ep.plan_id=original_plan AND ep.plan_revision=original_revision AND ep.channel_id=original_channel
      AND ep.currency=price->>'currency' AND ep.amount_minor=(price->>'amount_minor')::bigint)
     THEN RAISE EXCEPTION 'original_price_conflict'; END IF;
    INSERT INTO core_subscriptions.pancake_prices(plan_id,plan_revision,channel_id,store_id,product_id,product_type,billing_period,
      provider_plan_reference,return_url,checkout_region,checkout_language,tax_category)
    VALUES(original_plan,original_revision,original_channel,price->>'store_id',price->>'product_id',price->>'product_type',price->>'billing_period',
      price->>'provider_plan_reference',price->>'return_url',price->>'checkout_region',price->>'checkout_language',price->>'tax_category')
    ON CONFLICT DO NOTHING;
    SELECT to_jsonb(pp) INTO t FROM core_subscriptions.pancake_prices pp WHERE pp.plan_id=original_plan AND pp.plan_revision=original_revision AND pp.channel_id=original_channel;
    IF t IS DISTINCT FROM (price-ARRAY['currency','amount_minor']) || jsonb_build_object('plan_id',original_plan,'plan_revision',original_revision,'channel_id',original_channel)
     THEN RAISE EXCEPTION 'original_product_price_conflict'; END IF;
    INSERT INTO core_subscriptions.external_orders(id,actor_user_id,account_id,operation_key,plan_id,plan_revision,channel_id,
      provider,merchant,environment,currency,amount_minor,channel_binding,quoted_at,expires_at,confirmed_at)
    VALUES(original_order,(p->>'actor_user_id')::bigint,(p->>'account_id')::bigint,p->>'operation_key',original_plan,original_revision,original_channel,
      'waffo_pancake',b->>'merchant',b->>'environment',price->>'currency',(price->>'amount_minor')::bigint,b,
      (p->>'quoted_at')::bigint,(p->>'quoted_at')::bigint+1800,(p->>'quoted_at')::bigint);
    INSERT INTO core_subscriptions.pancake_orders(order_id,channel_id,provider_reference,binding,frozen_plan,legacy_source,provider_order_id,status,paid_period_start,paid_period_end)
    VALUES(original_order,original_channel,p->>'reference',b,p->'plan',p->'legacy_source',p->>'provider_order_id',
      CASE WHEN p->'legacy_source'->>'source_status'='success' THEN 'historical_paid' ELSE 'pending' END,coalesce((SELECT anchor FROM core_charging.subscriptions WHERE id=grant_id),0),(p->>'cutoff')::bigint);
   END IF;
   END IF;
  END LOOP;
  SET CONSTRAINTS ALL IMMEDIATE;
  SELECT ({state}) INTO after_state;
  IF before_state->'tables'-{changed} IS DISTINCT FROM after_state->'tables'-{changed}
   OR before_state->'sequences' IS DISTINCT FROM after_state->'sequences'
   THEN RAISE EXCEPTION 'original_unrelated_state_changed'; END IF;
  INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest)
  VALUES(m->>'migration_id',m->'financial_attestation'->>'source_archive_sha256',m->>'mapping_sha256',
    jsonb_build_object('scope',{F.literal(SCOPE)},'material',m,'post_state',after_state));
 END IF;
 {guard}
END $lmm_originals$;
COMMIT;
'''
    tag = '$lmm_originals_' + F.sha(sql.encode())[:24] + '$'
    require(tag not in sql, 'original_sql_delimiter_collision')
    # Source strings are SQL literals and may themselves contain template text.
    start = sql.index('DO $lmm_originals$'); end = sql.rindex('END $lmm_originals$;')
    return sql[:start] + sql[start:end].replace('DO $lmm_originals$', 'DO '+tag, 1) + 'END '+tag+';' + sql[end+len('END $lmm_originals$;'):]


def verify_approval(m, sql, approval):
    validate_material(m)
    require(approval == {'scope': SCOPE, 'decision': 'approve-offline-original-subscriptions',
        'material_sha256': F.digest(m), 'sql_sha256': F.sha(sql.encode()), 'review_sha256': F.digest(m['review'])}
        and sql == build_apply_sql(m), 'original_approval_mismatch')
