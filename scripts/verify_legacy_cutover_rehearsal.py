#!/usr/bin/env python3
# Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
"""Independently reconcile available Core imports against a local restored source.

Read-only Unix-socket connections under Backups only. Raw keys are hashed inside
PostgreSQL; only aggregate differences are reported. This checks the listed
fields, not live authentication, every financial feature, or cutover acceptance.
Exit 2 preserves a failed comparison or unresolved orphan/channel data as a gate.
"""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
from fractions import Fraction
import financial_import as F


def compare_rows(source, target, key='id'):
    sm={r[key]:r for r in source};tm={r[key]:r for r in target}
    F.require(len(sm)==len(source) and len(tm)==len(target),'duplicate_comparison_identifiers')
    missing=sm.keys()-tm.keys();extra=tm.keys()-sm.keys()
    different=sum(F.canonical(sm[k])!=F.canonical(tm[k]) for k in sm.keys()&tm.keys())
    return {'source_rows':len(source),'target_rows':len(target),'missing':len(missing),
        'extra':len(extra),'different':different,'passed':not(missing or extra or different)}


def normalized_quota(raw, divisor):
    F.require(type(raw) is int and raw>=0 and divisor>0,'invalid_quota_conversion')
    amount=Fraction(raw)/divisor
    return (2*amount.numerator+amount.denominator)//(2*amount.denominator)


def hero_sms_consistency(orders, ledgers):
    """Check each original order, so opposite errors cannot cancel globally."""
    by_id={o['id']:o for o in orders}
    F.require(len(by_id)==len(orders),'duplicate_sms_order_identifiers')
    totals={oid:{'reserve':0,'refund':0} for oid in by_id}
    errors={'orphan_ledger_rows':0,'wrong_ledger_owners':0,
        'invalid_ledger_entries':0,'reserve_mismatches':0,'refund_mismatches':0,
        'net_debit_mismatches':0,'completed_charge_mismatches':0,
        'cancelled_or_failed_debits':0}
    for row in ledgers:
        order=by_id.get(row['order_id'])
        if order is None:
            errors['orphan_ledger_rows']+=1
            continue
        errors['wrong_ledger_owners']+=row['user_id']!=order['user_id']
        kind,amount=row['entry_type'],row['amount_quota']
        if kind not in ('reserve','refund') or (kind=='reserve' and amount>0) or (kind=='refund' and amount<0):
            errors['invalid_ledger_entries']+=1
            continue
        totals[order['id']][kind]+=amount
    for oid,order in by_id.items():
        ledger=totals[oid]
        net=order['reserved_quota']-order['refunded_quota']
        errors['reserve_mismatches']+=-ledger['reserve']!=order['reserved_quota']
        errors['refund_mismatches']+=ledger['refund']!=order['refunded_quota']
        errors['net_debit_mismatches']+=-sum(ledger.values())!=net
        errors['completed_charge_mismatches']+=order['status']=='completed' and net!=order['charge_quota']
        errors['cancelled_or_failed_debits']+=order['status'] in ('cancelled','failed') and net!=0
    return {**errors,'source_orders':len(orders),'source_ledger_rows':len(ledgers),
        'passed':not any(errors.values())}



def reconcile_discount_facts(query, schema, archive_sha256, observed_at, compare):
    """Compare full original terms and held orders; this is not Native admission."""
    scope = "legacy-discount-facts.v1"
    selector = (f"i.manifest->>'scope'='{scope}' AND "
                f"i.source_archive_sha256='{archive_sha256}' AND "
                f"(i.manifest->>'observed_at')::bigint={observed_at}")
    source_users = [row['id'] for row in query(f'SELECT id FROM {schema}.users ORDER BY id', True)]
    compare('discount_facts_source_binding',
            [dict(id='receipt', source_namespace='lmm_api.lmm_prod_20260802.discounts',
                  source_user_ids=source_users, balance_effect='none',
                  native_discount_enabled_by_import=False, capacity_reserved_by_import=False)],
            query(f"SELECT 'receipt' AS id,manifest->>'source_namespace' AS source_namespace,"
                  f"manifest->'source_user_ids' AS source_user_ids,manifest->>'balance_effect' AS balance_effect,"
                  f"(manifest->>'native_discount_enabled_by_import')::boolean AS native_discount_enabled_by_import,"
                  f"(manifest->>'capacity_reserved_by_import')::boolean AS capacity_reserved_by_import "
                  f"FROM core_migration.import_runs i WHERE {selector}"))
    for table in ('discount_codes', 'discount_code_reservations'):
        value = ("(to_jsonb(d)-'code')||jsonb_build_object('code_sha256',"
                 "encode(sha256(convert_to(upper(btrim(d.code)),'UTF8')),'hex'))"
                 if table == 'discount_codes' else 'to_jsonb(d)')
        compare('discount_facts_' + table,
                query(f"SELECT id,md5(({value})::text) AS row_hash FROM {schema}.{table} d", True),
                query(f"SELECT (r->>'id')::bigint AS id,md5(r::text) AS row_hash "
                      f"FROM core_migration.import_runs i CROSS JOIN LATERAL "
                      f"jsonb_array_elements(i.manifest->'tables'->'{table}') r WHERE {selector}"))
    option_keys = "'QuotaPerUnit','LegacyPricingQuotaPerUnit','CreditsPerUSD','PublicCreditsPerUSD','USDExchangeRate','general_setting.quota_display_type','DisplayInCurrencyEnabled'"
    compare('discount_facts_quote_options',
            query(f"SELECT key AS id,md5(to_jsonb(o)::text) AS row_hash FROM "
                  f"(SELECT key,value FROM {schema}.options WHERE key IN({option_keys})) o", True),
            query(f"SELECT r->>'key' AS id,md5(r::text) AS row_hash FROM core_migration.import_runs i "
                  f"CROSS JOIN LATERAL jsonb_array_elements(i.manifest->'options') r WHERE {selector}"))
    orders = (f"SELECT id,user_id,trade_no,discount_code_id,status,payment_provider,failure_reason_code "
              f"FROM {schema}.top_ups WHERE discount_code_id>0 OR trade_no IN "
              f"(SELECT top_up_trade_no FROM {schema}.discount_code_reservations)")
    compare('discount_facts_related_orders',
            query(f"SELECT id,md5(to_jsonb(d)::text) AS row_hash FROM ({orders}) d", True),
            query(f"SELECT (r->>'id')::bigint AS id,md5(r::text) AS row_hash "
                  f"FROM core_migration.import_runs i CROSS JOIN LATERAL "
                  f"jsonb_array_elements(i.manifest->'tables'->'discount_top_ups') r WHERE {selector}"))
    payable = "(t.status='pending' OR(t.status='failed' AND t.payment_provider='waffo_pancake' AND t.failure_reason_code='checkout_timeout'))"
    closed = "(t.status='expired' OR(t.status='failed' AND(coalesce(t.payment_provider,'')<>'waffo_pancake' OR(t.failure_reason_code IS NOT NULL AND t.failure_reason_code<>'checkout_timeout'))))"
    holds = (f"SELECT r.top_up_trade_no AS id,r.discount_code_id,t.id AS source_order_id,"
             f"r.id AS source_reservation_id,coalesce(t.user_id,r.user_id) AS source_user_id "
             f"FROM {schema}.discount_code_reservations r LEFT JOIN {schema}.top_ups t ON t.trade_no=r.top_up_trade_no "
             f"WHERE(r.status='reserved' AND NOT coalesce({closed},false)) OR(r.status='released' AND {payable}) "
             f"UNION ALL SELECT t.trade_no,t.discount_code_id,t.id,NULL::bigint,t.user_id "
             f"FROM {schema}.top_ups t WHERE t.discount_code_id>0 AND {payable} AND NOT EXISTS "
             f"(SELECT FROM {schema}.discount_code_reservations r WHERE r.top_up_trade_no=t.trade_no)")
    compare('discount_facts_capacity_holds', query(holds, True),
            query(f"SELECT r->>'top_up_trade_no' AS id,(r->>'discount_code_id')::bigint AS discount_code_id,"
                  f"(r->>'source_order_id')::bigint AS source_order_id,"
                  f"(r->>'source_reservation_id')::bigint AS source_reservation_id,"
                  f"(r->>'source_user_id')::bigint AS source_user_id "
                  f"FROM core_migration.import_runs i CROSS JOIN LATERAL "
                  f"jsonb_array_elements(i.manifest->'capacity_holds') r WHERE {selector}"))


def reconciliation_status(checks, gaps):
    if not all(check['passed'] for check in checks.values()):
        return 'FAILED'
    return 'SCOPED_PASS_WITH_GAPS' if any(gaps.values()) else 'SCOPED_PASS'


def read_rows(args, sql, source=False):
    args.query_number+=1
    env={k:v for k,v in os.environ.items() if not k.startswith('PG')}
    env.update(PGHOST=str(args.socket),PGPORT=str(args.port),
        PGDATABASE=args.source_db if source else args.target_db,
        PGUSER=args.source_user if source else args.target_login,PGCONNECT_TIMEOUT='5',
        PGOPTIONS='-c default_transaction_read_only=on -c timezone=UTC'+('' if source else ' -c role='+args.target_owner))
    command=['psql','-X','--no-password','-qAt','-v','ON_ERROR_STOP=1']
    try:
        p=subprocess.run(command,input="BEGIN READ ONLY; SET LOCAL statement_timeout='90s'; SELECT coalesce(jsonb_agg(to_jsonb(q)),'[]'::jsonb) FROM ("+sql+") q; ROLLBACK;",text=True,capture_output=True,env=env,timeout=100)
    except subprocess.TimeoutExpired:
        raise F.Error('verification_query_timeout') from None
    if p.returncode:
        F.write_private(args.report.parent,args.report.name+'.query-'+str(args.query_number)+'.error.private',p.stderr)
        raise F.Error('verification_query_failed_private_diagnostics_retained')
    return json.loads(p.stdout)


def reconcile(args):
    schema=args.source_schema
    checks={}
    query=lambda sql,source=False:read_rows(args,sql,source)
    def compare(name,source,target,key='id'):
        checks[name]=compare_rows(source,target,key)
    aliases=json.loads(F.IDENTITY.private_backup_path(args.aliases).read_text())
    receipt_path=args.aliases.with_name('credential-mapping-receipt.private.json')
    F.require(aliases.get('version')==1 and aliases.get('scope')=='legacy-api-key-wire-aliases-v1'
        and aliases.get('source_archive_sha256')==args.archive_sha256
        and F.IDENTITY.sha256_file(F.IDENTITY.private_backup_path(receipt_path))==aliases.get('credential_receipt_sha256'),
        'alias_source_receipt_mismatch')
    identity=query("SELECT current_database() AS database,(SELECT oid::bigint FROM pg_database WHERE datname=current_database()) AS database_oid,current_setting('lmm.migration_instance_nonce',true) AS instance_nonce")[0]
    F.require(aliases.get('target')==identity,'alias_target_binding_mismatch')
    opening=query('SELECT source_archive_sha256,observed_at FROM core_billing.opening_import_runs')
    F.require(len(opening)==1 and opening[0]=={'source_archive_sha256':args.archive_sha256,'observed_at':args.observed_at},'opening_source_binding_mismatch')
    # Compare amounts, registration time and user authority independently of the planners.
    s=query(f"SELECT id,coalesce(quota,0) AS available,coalesce(aff_quota,0) AS affiliate,coalesce(created_at,0) AS registered_at,CASE role WHEN 1 THEN 'user' WHEN 10 THEN 'admin' WHEN 100 THEN 'superadmin' END AS role,(status=1 AND deleted_at IS NULL) AS active FROM {schema}.users",True)
    t=query("SELECT u.id,coalesce(sum(s.balance_units) FILTER(WHERE l.bucket IN('wallet','debt')),0) AS available,coalesce(sum(s.balance_units) FILTER(WHERE l.bucket='affiliate'),0) AS affiliate,floor(extract(epoch FROM a.created_at))::bigint AS registered_at,u.platform_role AS role,u.active FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id LEFT JOIN core_billing.ledger_accounts l ON l.owner_account_id=a.id LEFT JOIN core_billing.balance_state s ON s.ledger_account_id=l.id GROUP BY u.id,a.created_at")
    compare('account_balances_registration_and_roles',s,t)
    compare('historical_account_counters',query(f'SELECT id,coalesce(used_quota,0) AS used_quota,coalesce(request_count,0) AS request_count,coalesce(aff_history,0) AS aff_history FROM {schema}.users',True),query('SELECT user_id AS id,used_quota,request_count,aff_history FROM core_billing.account_opening_counters'))
    # Key quotas and lifecycle metadata are compared without returning raw keys.
    ordinary="(length(t.key) BETWEEN 32 AND 256 AND t.key ~ '^[A-Za-z0-9_]+$' AND lower(t.key) !~ 'lmm_|oauth_managed_' AND lower(t.key) !~ '^lmm[krsifto]_')"
    managed="(coalesce(t.oauth_managed,false) OR coalesce(t.creation_source,'')='assistant_runtime' OR NOT " + ordinary + ")"
    s=query(f"SELECT t.id,t.user_id,t.user_id AS owner_account_id,greatest(coalesce(u.auth_version,1),1) AS user_version,encode(sha256(convert_to(t.key,'UTF8')),'hex') AS digest,coalesce(t.used_quota,0) AS used,coalesce(t.used_quota,0)+coalesce(t.remain_quota,0) AS displayed_limit,t.unlimited_quota AS unlimited,t.status=1 AS enabled,(t.deleted_at IS NOT NULL OR {managed}) AS revoked,CASE WHEN t.expired_time IS NULL OR t.expired_time=-1 THEN 253402300799 ELSE t.expired_time+1 END AS expires_at FROM {schema}.tokens t JOIN {schema}.users u ON u.id=t.user_id",True)
    t=query("SELECT c.id,c.user_id,c.owner_account_id,c.user_version,encode(c.digest,'hex') AS digest,o.consumed_credits AS used,CASE WHEN q.unlimited THEN q.display_limit_credits ELSE q.limit_credits END AS displayed_limit,q.unlimited,k.enabled,c.revoked_at IS NOT NULL AS revoked,floor(extract(epoch FROM c.expires_at))::bigint AS expires_at FROM core_identity.credentials c JOIN core_identity.api_key_settings k ON k.credential_id=c.id JOIN core_charging.budgets q ON q.key_id=c.id AND q.scope='key' JOIN core_charging.budget_opening_consumption o ON o.key_id=c.id AND o.scope='key' WHERE c.kind='api_key'")
    compare('api_key_owner_digest_quota_and_lifecycle',s,t)
    s=query(f"SELECT t.id AS source_key_id,t.id AS target_credential_id,t.user_id AS target_user_id,t.user_id AS target_account_id,greatest(coalesce(u.auth_version,1),1) AS credential_user_version,encode(sha256(convert_to(t.key,'UTF8')),'hex') AS source_secret_sha256,encode(sha256(convert_to('sk-'||t.key,'UTF8')),'hex') AS sk_alias_sha256,{managed} AS revoked_rekey_required FROM {schema}.tokens t JOIN {schema}.users u ON u.id=t.user_id",True)
    fields=('source_key_id','target_credential_id','target_user_id','target_account_id','credential_user_version','source_secret_sha256','sk_alias_sha256')
    t=[{**{k:r[k] for k in fields},'revoked_rekey_required':r.get('revoked_rekey_required',False)} for r in aliases['keys']]
    compare('raw_and_sk_alias_bindings',s,t,'source_key_id')
    # Preserve every original check-in, including deleted-user history. This
    # archive is not a native claim and must never replay an already-opened award.
    compare('checkin_original_history',
        query(f'SELECT id,user_id,checkin_date,quota_awarded,created_at FROM {schema}.checkins',True),
        query("SELECT (r->'record'->>'id')::bigint AS id,(r->'record'->>'user_id')::bigint AS user_id,r->'record'->>'checkin_date' AS checkin_date,(r->'record'->>'quota_awarded')::bigint AS quota_awarded,(r->'record'->>'created_at')::bigint AS created_at FROM core_migration.import_runs i CROSS JOIN LATERAL jsonb_array_elements(i.manifest->'rows') r WHERE i.manifest->>'scope'='checkin-history.v1'"))
    checkin_keys="('checkin_setting.enabled','checkin_setting.min_quota','checkin_setting.max_quota','checkin_setting.level_multipliers')"
    compare('checkin_original_reward_options_archive',
        query(f'SELECT key AS id,value FROM {schema}.options WHERE key IN {checkin_keys}',True),
        query("SELECT e.key AS id,e.value FROM core_migration.import_runs i CROSS JOIN LATERAL jsonb_each_text(i.manifest->'settings') e WHERE i.manifest->>'scope'='checkin-history.v1'"))
    receipts=query("SELECT source_archive_sha256,manifest->>'source_namespace' AS namespace,(manifest->>'observed_at')::bigint AS observed_at,manifest->>'balance_effect' AS balance_effect FROM core_migration.import_runs WHERE manifest->>'scope'='checkin-history.v1'")
    checks['checkin_history_source_binding']={'passed':receipts==[{'source_archive_sha256':args.archive_sha256,'namespace':'lmm_api.lmm_prod_20260802.checkins','observed_at':args.observed_at,'balance_effect':'none'}]}
    # Original SMS price strings, encrypted payloads and monetary ledger fields
    # must all survive. Compare PG canonical JSON hashes, not planner summaries.
    for table,cast in [('hero_sms_sms_orders',''),('hero_sms_sms_quota_ledgers','::bigint')]:
        compare(table+'_original_all_fields',
            query(f"SELECT id,encode(sha256(convert_to(to_jsonb(o)::text,'UTF8')),'hex') AS projection FROM {schema}.{table} o",True),
            query(f"SELECT (r->>'id'){cast} AS id,encode(sha256(convert_to(r::text,'UTF8')),'hex') AS projection FROM core_migration.import_runs i CROSS JOIN LATERAL jsonb_array_elements(i.manifest->'tables'->'{table}') r WHERE i.manifest->>'scope'='hero-sms-history.v1'"))
    sms_receipts=query("SELECT source_archive_sha256,manifest->>'source_namespace' AS namespace,(manifest->>'observed_at')::bigint AS observed_at,manifest->>'balance_effect' AS balance_effect FROM core_migration.import_runs WHERE manifest->>'scope'='hero-sms-history.v1'")
    checks['hero_sms_history_source_binding']={'passed':sms_receipts==[{'source_archive_sha256':args.archive_sha256,'namespace':'lmm_api.lmm_prod_20260802.hero_sms_sms','observed_at':args.observed_at,'balance_effect':'none'}]}
    # Terminal transfers retain real original history; never invent Core settlements.
    s=query(f"SELECT id,sender_id AS owner_user,CASE WHEN status='claimed' THEN recipient_id END AS recipient_user,quota AS amount,status AS state,created_at,CASE WHEN status='claimed' THEN claimed_at ELSE cancelled_at END AS finished_at FROM {schema}.wallet_transfers WHERE status IN('claimed','cancelled')",True)
    t=query('SELECT id,owner_user,recipient_user,amount,state,created_at,finished_at FROM core_wallet.legacy_transfer_history')
    compare('terminal_wallet_transfer_history',s,t)
    s=query(f"SELECT id,owner_user_id,payer_account_id,escrow_quota AS escrow_units,status FROM (SELECT p.*,p.owner_user_id AS payer_account_id FROM {schema}.open_source_bounty_projects p) x WHERE escrow_quota>0",True)
    t=query('SELECT source_project_id AS id,owner_user_id,payer_account_id,escrow_units,status FROM core_billing.legacy_escrow_projects')
    compare('bounty_escrow_original_projects',s,t)
    observed=args.observed_at
    s=query(f"SELECT c.id,c.project_id AS source_project_id,c.participant_user_id,c.participant_user_id AS recipient_account_id,c.reward_quota AS source_reward_quota,CASE WHEN c.status='rejected' AND c.rejected_at<={observed}-604800 THEN 'historical_rejection_guard_only' ELSE 'active_future_reward' END AS rights_status,CASE WHEN c.status='rejected' AND c.rejected_at<={observed}-604800 THEN NULL ELSE c.reward_quota END AS payout_units,c.status,c.rejected_at,c.paid_at FROM {schema}.open_source_bounty_challenges c JOIN {schema}.open_source_bounty_projects p ON p.id=c.project_id WHERE p.escrow_quota>0 AND coalesce(c.paid_at,0)=0 AND c.status IN('accepted','submitted','rejected')",True)
    t=query('SELECT source_challenge_id AS id,source_project_id,participant_user_id,recipient_account_id,source_reward_quota,rights_status,payout_units,status,rejected_at,paid_at FROM core_billing.legacy_escrow_claims')
    compare('bounty_claims_rights_and_rejection_guards',s,t)
    compare('bounty_initial_remaining_escrow',query(f'SELECT id,escrow_quota AS remaining_units FROM {schema}.open_source_bounty_projects WHERE escrow_quota>0',True),query('SELECT source_project_id AS id,remaining_units FROM core_billing.legacy_escrow_state'))
    # Redeemability follows the snapshot evaluation time; spent codes remain revoked.
    observed=args.observed_at
    s=query(f"SELECT encode(sha256(convert_to(trim(key),'UTF8')),'hex') AS id,quota AS award_units,NOT(status=1 AND deleted_at IS NULL AND (coalesce(expired_time,0)=0 OR expired_time>{observed})) AS revoked FROM {schema}.redemptions WHERE reward_type='quota'",True)
    t=query("SELECT encode(c.digest,'hex') AS id,p.award_units,c.revoked FROM core_promotions.codes c JOIN core_promotions.campaigns p ON p.id=c.campaign_id")
    compare('redemption_amounts_and_revocation',s,t)
    # Source-linked grant shape and consumed units; quote/order linkage is a separate gap.
    s=query(f"SELECT id,user_id AS account_id,start_time AS anchor,end_time AS ends_at,amount_total AS amount_total,amount_used AS used FROM {schema}.user_subscriptions",True)
    t=query("SELECT split_part(s.grant_reference,':',3)::bigint AS id,s.account_id,s.anchor,s.ends_at,CASE WHEN s.unlimited THEN 0 ELSE s.limit_credits END AS amount_total,o.consumed_credits AS used FROM core_charging.subscriptions s JOIN core_charging.subscription_opening_consumption o ON o.subscription_id=s.id WHERE s.grant_reference LIKE 'legacy-sub:%'")
    compare('subscription_original_grants_and_consumption',s,t)
    # Compare each mapped historical log across 15 exported fields using the same PG
    # JSON numeric/text encoding on each side. Extra source-only metadata is excluded.
    s_fields="l.user_id,l.user_id,CASE WHEN t.user_id=l.user_id THEN l.token_id END,l.created_at,l.type,l.quota,l.prompt_tokens,l.completion_tokens,coalesce(l.username,''),coalesce(l.token_name,''),coalesce(l.model_name,''),coalesce(l.channel_id,0),coalesce(l.\"group\",''),coalesce(l.request_id,''),coalesce(l.upstream_request_id,'')"
    t_fields="user_id,owner_account_id,token_id,created_at,log_type,quota,prompt_tokens,completion_tokens,username,token_name,model_name,channel_id,group_name,request_id,upstream_request_id"
    s=query(f"SELECT l.id,encode(sha256(convert_to(jsonb_build_array({s_fields})::text,'UTF8')),'hex') AS projection FROM {schema}.logs l JOIN {schema}.users u ON u.id=l.user_id LEFT JOIN {schema}.tokens t ON t.id=l.token_id",True)
    t=query(f"SELECT source_log_id AS id,encode(sha256(convert_to(jsonb_build_array({t_fields})::text,'UTF8')),'hex') AS projection FROM core_history.log_records")
    compare('mapped_log_field_projections',s,t)
    # Quota conversion uses the independently captured rebase divisor and exact
    # rational half-away-from-zero rounding, never an average or guessed unit.
    rebases=query(f"SELECT plan->>'divisor' AS divisor,plan->>'rounding' AS rounding,(plan->>'snapshot_at')::bigint AS snapshot_at,floor(extract(epoch FROM applied_at))::bigint AS applied_at FROM {schema}.wallet_credit_rebases",True)
    F.require(len(rebases)==1 and rebases[0]['rounding']=='half-away-from-zero','unsupported_independent_quota_conversion')
    r=rebases[0];divisor=Fraction(r['divisor'])
    s=query(f'SELECT q.id,q.user_id,q.user_id AS owner_account_id,coalesce(q.quota,0) AS raw_quota,q.created_at,coalesce(q.count,0) AS request_count,coalesce(q.token_used,0) AS token_used FROM {schema}.quota_data q JOIN {schema}.users u ON u.id=q.user_id',True)
    for row in s:
     if row['created_at']>=r['applied_at']:row.update(quota_credits=row['raw_quota'],credits_basis='current_units')
     elif row['created_at']+3600<=r['snapshot_at']:
      row.update(quota_credits=normalized_quota(row['raw_quota'],divisor),credits_basis='rebased_legacy_units')
     else:raise F.Error('unproved_quota_bucket_unit')
    t=query('SELECT source_row_id AS id,user_id,owner_account_id,raw_quota,created_at,request_count,token_used,quota_credits,credits_basis FROM core_history.quota_aggregates')
    compare('mapped_quota_counts_raw_and_normalized_units',s,t)
    ledger=query("SELECT (SELECT coalesce(sum(balance_units),0) FROM core_billing.balance_state) AS global_balance_sum,(SELECT count(*) FROM (SELECT journal_id FROM core_billing.ledger_entries GROUP BY journal_id HAVING sum(delta_units)<>0) x) AS unbalanced_journals,(SELECT count(*) FROM core_billing.balance_state s WHERE s.balance_units<>coalesce((SELECT sum(e.delta_units) FROM core_billing.ledger_entries e WHERE e.ledger_account_id=s.ledger_account_id),0)) AS balances_disagree_with_entries,(SELECT count(*) FROM core_billing.balance_state s WHERE s.revision<>coalesce((SELECT max(e.balance_revision) FROM core_billing.ledger_entries e WHERE e.ledger_account_id=s.ledger_account_id),0)) AS revisions_disagree_with_entries,(SELECT count(*) FROM core_billing.ledger_journals j WHERE j.kind<>'opening' AND (coalesce((SELECT sum(e.delta_units) FROM core_billing.ledger_entries e WHERE e.journal_id=j.id AND e.ledger_account_id=j.payer_id),0)<>-j.amount_units OR coalesce((SELECT sum(e.delta_units) FROM core_billing.ledger_entries e WHERE e.journal_id=j.id AND e.ledger_account_id=j.payee_id),0)<>j.amount_units)) AS nonopening_payer_payee_mismatches")[0]
    checks['ledger_invariants']={**ledger,'passed':all(v==0 for v in ledger.values())}
    orders=query(f'SELECT id,user_id,status,reserved_quota,refunded_quota,charge_quota FROM {schema}.hero_sms_sms_orders',True)
    sms_ledgers=query(f'SELECT order_id,user_id,entry_type,amount_quota FROM {schema}.hero_sms_sms_quota_ledgers',True)
    checks['source_hero_sms_order_ledger_consistency']=hero_sms_consistency(orders,sms_ledgers)
    gaps=query(f"SELECT (SELECT count(*) FROM {schema}.logs l WHERE NOT EXISTS(SELECT FROM {schema}.users u WHERE u.id=l.user_id)) AS orphan_log_rows,(SELECT count(*) FROM {schema}.quota_data q WHERE NOT EXISTS(SELECT FROM {schema}.users u WHERE u.id=q.user_id)) AS orphan_quota_rows,1 AS checkin_native_reward_contract_not_verified",True)[0]
    channel_ids={r['id'] for r in query(f'SELECT id FROM {schema}.channels',True)}
    gaps['channel_identities_not_imported']=len(channel_ids-{r['id'] for r in query('SELECT id FROM core_channels.identities')})
    gaps.update(query(f"SELECT (SELECT count(*) FROM {schema}.discount_codes) AS source_discount_codes_not_covered_by_this_verifier,(SELECT count(*) FROM {schema}.discount_code_reservations) AS source_discount_reservations_not_covered_by_this_verifier,(SELECT count(*) FROM {schema}.referral_rewards) AS source_referral_rewards_not_covered_by_this_verifier,(SELECT count(*) FROM {schema}.referral_rewards WHERE status='earned' AND stage='awaiting_first_top_up') AS referral_rewards_awaiting_first_top_up_not_imported,(SELECT count(*) FROM {schema}.public_relay_contributions) AS public_relay_contributions_not_imported,(SELECT count(*) FROM {schema}.hero_sms_email_orders) AS hero_sms_email_orders_not_imported",True)[0])
    gaps['hero_sms_native_operations_not_verified']=1
    gaps['hero_sms_encrypted_payload_read_not_verified']=1
    # Failed Waffo checkout timeouts remain payable under the original lifecycle.
    late = "status='failed' AND payment_provider='waffo_pancake' AND failure_reason_code='checkout_timeout'"
    compare('late_payable_topup_liabilities',
        query(f"SELECT t.id,t.user_id,t.status,t.payment_provider,t.failure_reason_code,t.money::text AS money,t.credited_quota,t.discount_code_id,t.discount_percent,t.discount_qualifying_amount,t.discount_qualifying_unit,t.settlement_currency,t.expected_amount_micros,false AS spendable,0::bigint AS spendable_units FROM {schema}.top_ups t WHERE {late} AND EXISTS(SELECT FROM {schema}.users u WHERE u.id=t.user_id)",True),
        query(f"SELECT source_row_key::bigint AS id,owner_user_id AS user_id,status,payload_json->>'payment_provider' AS payment_provider,payload_json->>'failure_reason_code' AS failure_reason_code,payload_json->>'money' AS money,(payload_json->>'credited_quota')::bigint AS credited_quota,(payload_json->>'discount_code_id')::bigint AS discount_code_id,(payload_json->>'discount_percent')::bigint AS discount_percent,payload_json->>'discount_qualifying_amount' AS discount_qualifying_amount,payload_json->>'discount_qualifying_unit' AS discount_qualifying_unit,payload_json->>'settlement_currency' AS settlement_currency,(payload_json->>'expected_amount_micros')::bigint AS expected_amount_micros,spendable,spendable_units FROM core_billing.legacy_pending_liabilities WHERE source_table='top_ups' AND source_archive_sha256='{args.archive_sha256}' AND observed_at={args.observed_at} AND settlement_state='unknown' AND payload_json->>'status'='failed' AND payload_json->>'payment_provider'='waffo_pancake' AND payload_json->>'failure_reason_code'='checkout_timeout'"))
    gaps.update(query(f"SELECT count(*) AS payable_topup_orphan_rows_not_mapped FROM {schema}.top_ups t WHERE ({late}) AND NOT EXISTS(SELECT FROM {schema}.users u WHERE u.id=t.user_id)",True)[0])
    gaps['discount_capacity_holds_not_imported']=query(f"WITH payable AS(SELECT trade_no,discount_code_id FROM {schema}.top_ups WHERE status='pending' OR(status='failed' AND payment_provider='waffo_pancake' AND failure_reason_code='checkout_timeout')),closed AS(SELECT trade_no FROM {schema}.top_ups WHERE trade_no IS NOT NULL AND(status='expired' OR(status='failed' AND(COALESCE(payment_provider,'')<>'waffo_pancake' OR failure_reason_code<>'checkout_timeout')))) SELECT (SELECT count(*) FROM {schema}.discount_code_reservations r WHERE(r.status='reserved' AND r.top_up_trade_no NOT IN(SELECT trade_no FROM closed)) OR(r.status='released' AND r.top_up_trade_no IN(SELECT trade_no FROM payable)))+(SELECT count(*) FROM payable t WHERE discount_code_id>0 AND NOT EXISTS(SELECT FROM {schema}.discount_code_reservations r WHERE r.top_up_trade_no=t.trade_no)) AS holds",True)[0]['holds']
    gaps['legacy_late_payment_callback_not_verified']=1
    basis_scope="legacy-topup-settlement-facts.v1"
    basis_filter=f"i.manifest->>'scope'='{basis_scope}' AND i.source_archive_sha256='{args.archive_sha256}' AND (i.manifest->>'observed_at')::bigint={args.observed_at}"
    compare('legacy_settlement_source_orders',
        query(f"SELECT id,md5((to_jsonb(t)||jsonb_build_object('money',t.money::text))::text) AS row_hash FROM {schema}.top_ups t WHERE status='pending' OR(status='failed' AND payment_provider='waffo_pancake' AND failure_reason_code='checkout_timeout')",True),
        query(f"SELECT (r->>'id')::bigint AS id,md5(r::text) AS row_hash FROM core_migration.import_runs i CROSS JOIN LATERAL jsonb_array_elements(i.manifest->'tables'->'top_ups') r WHERE {basis_filter}"))
    compare('legacy_settlement_parent_audits',
        query(f"SELECT migration_id AS id,md5(to_jsonb(p)::text) AS row_hash FROM {schema}.wallet_credit_rebases p",True),
        query(f"SELECT r->>'migration_id' AS id,md5(r::text) AS row_hash FROM core_migration.import_runs i CROSS JOIN LATERAL jsonb_array_elements(i.manifest->'tables'->'wallet_credit_rebases') r WHERE {basis_filter}"))
    compare('legacy_settlement_credit_options',
        query(f"SELECT key AS id,md5(to_jsonb(o)::text) AS row_hash FROM {schema}.options o WHERE key IN('CreditsPerUSD','LegacyPricingQuotaPerUnit','QuotaPerUnit','USDExchangeRate','TopUpPlatformUnitsPerCNY','PublicCreditsPerUSD')",True),
        query(f"SELECT r->>'key' AS id,md5(r::text) AS row_hash FROM core_migration.import_runs i CROSS JOIN LATERAL jsonb_array_elements(i.manifest->'tables'->'credit_options') r WHERE {basis_filter}"))
    basis=query(f"SELECT (manifest->>'original_quota_rejected_count')::bigint AS rejected FROM core_migration.import_runs i WHERE {basis_filter}")
    gaps['legacy_original_quota_rejections_require_handling']=basis[0]['rejected'] if len(basis)==1 else 1
    gaps['legacy_payment_channel_binding_not_verified']=1
    reconcile_discount_facts(query, schema, args.archive_sha256, args.observed_at, compare)
    gaps['discount_native_quote_reservation_and_refund_not_verified']=1
    for table in ('discount_codes','discount_code_reservations'):
        if checks['discount_facts_'+table]['passed']:
            gaps['source_'+('discount_reservations' if table=='discount_code_reservations' else table)+'_not_covered_by_this_verifier']=0
    # A source ledger check proves old-book consistency, never target migration.
    status=reconciliation_status(checks,gaps)
    return {'scope':'read-only-available-core-phase-reconciliation','status':status,
        'checks':checks,'source_gaps':gaps,'http_authentication_tested':False,
        'inputs':{'source_archive_sha256':args.archive_sha256,'observed_at':args.observed_at,
            'aliases_sha256':F.IDENTITY.sha256_file(args.aliases),
            'verifier_sha256':F.IDENTITY.sha256_file(Path(__file__))},
        'full_business_acceptance_complete':False,'production_accessed':False}


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--socket',required=True,type=Path)
    p.add_argument('--port',required=True,type=int)
    p.add_argument('--source-db',required=True)
    p.add_argument('--source-user',required=True)
    p.add_argument('--source-schema',required=True)
    p.add_argument('--target-db',required=True)
    p.add_argument('--target-login',required=True)
    p.add_argument('--target-owner',required=True)
    p.add_argument('--archive-sha256',required=True)
    p.add_argument('--observed-at',required=True,type=int)
    p.add_argument('--aliases',required=True,type=Path)
    p.add_argument('--report',required=True,type=Path)
    args=p.parse_args()
    try:
        args.socket=F.IDENTITY.private_backup_path(args.socket,directory=True)
        args.aliases=F.IDENTITY.private_backup_path(args.aliases)
        args.report=F.IDENTITY.private_backup_path(args.report.parent,directory=True)/args.report.name
        F.require(not args.report.exists() and args.source_db!=args.target_db,'new_report_and_distinct_databases_required')
        F.require(all(re.fullmatch(r'[a-z_][a-z_0-9]{0,62}',v) for v in (args.source_db,args.source_user,args.source_schema,args.target_db,args.target_login,args.target_owner)),'invalid_database_identifier')
        F.require(1<=args.port<=65535 and 0<args.observed_at<=253402300799,'invalid_port_or_observation_time')
        F.checked_hash(args.archive_sha256)
        args.query_number=0
        result=reconcile(args)
        F.write_private(args.report.parent,args.report.name,json.dumps(result,sort_keys=True,indent=2)+'\n')
        print(json.dumps(result,sort_keys=True))
        return 0 if result['status']=='SCOPED_PASS' else 2
    except (F.Error,F.IDENTITY.MappingError) as error:
        print('error: '+str(error))
        return 2


if __name__=='__main__':
    raise SystemExit(main())
