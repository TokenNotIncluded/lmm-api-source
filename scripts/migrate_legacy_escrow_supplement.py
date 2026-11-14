#!/usr/bin/env python3
"""Restore reviewed frozen escrow after partial wallet opening, without reopening wallets.

Calls the installed owner-only ledger function. No DDL, payout, wallet debit,
service fence or automatic retry. Original candidate and wallet receipts stay intact.
"""
import argparse
import copy
import os
from pathlib import Path
import sys
import time
import uuid
import migrate_legacy_available_wallet as W

SCOPE = 'financial-escrow-rights-supplement-v1'


def prepare(candidate, wallet, unit, policy, migration_id):
    W.require(candidate['blockers'] == [] and candidate['source_unit_evidence_bound'] is True
              and candidate['post_parent_claim_evidence_bound'] is True
              and candidate['current_amounts_rescaled'] is False, 'escrow_candidate_not_closed')
    W.require(wallet['scope'] == W.SCOPE and wallet['policy'] == W.POLICY,
              'explicit_partial_wallet_required')
    archive = wallet['source_archive_sha256']
    W.require(candidate['source_archive_sha256'] == unit['source_archive_sha256'] == archive
              and candidate['mapping_sha256'] == unit['mapping_sha256'] == wallet['source_plan_mapping_sha256']
              and unit['source_export_sha256'] == wallet['source_files']['source_export']['sha256'],
              'escrow_source_binding_mismatch')
    measured = policy['current_measured_unit_evidence']
    options = {x['key']: x['value'] for x in measured['observed_options']}
    W.require(unit['unit'] == 'credit_500k_usd' and unit['credits_per_usd'] == 500000
              and options.get('CreditsPerUSD') == options.get('QuotaPerUnit') == '500000'
              and policy['conversion_policy'] == 'preserve-exact-parent-after-integers'
              and policy['new_conversion_applied'] is False
              and policy['parent_binding']['reconstructed_compact_parent_hash_matches'] is True
              and policy['parent_binding']['regenerated_entity_rules_match'] is True,
              'escrow_current_unit_or_parent_basis_missing')
    batch = copy.deepcopy(candidate['batch_candidate'])
    W.require(batch['unit'] == unit['unit'] and batch['source_archive_sha256'] == archive
              and batch['observed_at'] == wallet['observed_at'], 'escrow_observation_mismatch')
    identities = {x['target_user_id']: x for x in wallet['positions']}
    involved = set()
    for project in batch['projects']:
        W.require(identities[project['owner_user_id']]['account_id'] == project['payer_account_id'],
                  'escrow_payer_mapping_mismatch')
        involved.add(project['owner_user_id'])
        for claim in project['claims']:
            W.require(identities[claim['participant_user_id']]['account_id'] == claim['recipient_account_id'],
                      'escrow_recipient_mapping_mismatch')
            involved.add(claim['participant_user_id'])
    review = {'scope': SCOPE, 'decision': 'approved-phased-frozen-escrow-restoration-no-wallet-debit',
              'candidate_sha256': candidate['candidate_sha256'], 'candidate_mapping_sha256': candidate['mapping_sha256'],
              'unit_evidence': unit, 'policy_evidence': policy,
              'wallet_migration_id': wallet['migration_id'], 'wallet_mapping_sha256': wallet['mapping_sha256'],
              'all_financial_rights_complete': False, 'automatic_payout_authorized': False}
    # A new, explicitly reviewed bridge to the actual partial opening. This does
    # not modify or relabel the historical candidate's original mapping/receipt.
    batch.update(migration_id=wallet['migration_id'], mapping_sha256=wallet['mapping_sha256'],
                 review_proof_sha256=W.digest(review))
    return {'scope': SCOPE, 'migration_id': migration_id, 'source_archive_sha256': archive,
            'target': wallet['target'], 'connection': wallet['connection'], 'review': review,
            'request': batch, 'identities': [identities[i] for i in sorted(involved)]}


def build_sql(m):
    W.require(m['scope'] == SCOPE and m['request']['review_proof_sha256'] == W.digest(m['review']),
              'escrow_supplement_review_changed')
    t, request = m['target'], m['request']
    L = W.literal
    manifest = {'scope': SCOPE, 'partial': True, 'full_financial_import_complete': False, 'material': m}
    balances = "SELECT coalesce(jsonb_agg(jsonb_build_array(a.id,b.balance_units) ORDER BY a.id),'[]'::jsonb) FROM core_billing.ledger_accounts a JOIN core_billing.balance_state b ON b.ledger_account_id=a.id WHERE a.bucket IN ('wallet','debt','affiliate')"
    checks = ''
    for x in m['identities']:
        checks += f"""IF NOT EXISTS(SELECT 1 FROM core_identity.users u JOIN core_billing.opening_imports o ON o.account_id=u.personal_account_id WHERE u.id={x['target_user_id']} AND u.personal_account_id={x['account_id']} AND o.migration_id={L(request['migration_id'])} AND o.source_entity_key={L('legacy.users/'+str(x['source_user_id']))}) THEN RAISE EXCEPTION 'escrow original identity opening mismatch'; END IF;\n"""
    for p in request['projects']:
        checks += f"""IF NOT EXISTS(SELECT 1 FROM core_billing.legacy_escrow_projects WHERE migration_id={L(request['migration_id'])} AND source_project_id={p['source_project_id']} AND source={L(p)}::jsonb) THEN RAISE EXCEPTION 'escrow project exact readback mismatch'; END IF;\n"""
        for table, key, records in [('legacy_escrow_claims','source_challenge_id',p['claims']),('legacy_escrow_disputes','source_dispute_id',p['disputes'])]:
            checks += f"IF (SELECT count(*) FROM core_billing.{table} WHERE migration_id={L(request['migration_id'])} AND source_project_id={p['source_project_id']})<>{len(records)} THEN RAISE EXCEPTION 'escrow rights coverage mismatch'; END IF;\n"
            for r in records:
                checks += f"IF NOT EXISTS(SELECT 1 FROM core_billing.{table} WHERE migration_id={L(request['migration_id'])} AND {key}={r[key]} AND source={L(r)}::jsonb) THEN RAISE EXCEPTION 'escrow right exact readback mismatch'; END IF;\n"
    return f"""BEGIN ISOLATION LEVEL SERIALIZABLE;
SET LOCAL synchronous_commit=on;
SET LOCAL lock_timeout='5s'; SET LOCAL statement_timeout='60s';
SELECT pg_advisory_xact_lock(741803121);
LOCK TABLE core_billing.ledger_accounts,core_billing.balance_state,core_billing.ledger_journals,core_billing.ledger_entries IN SHARE ROW EXCLUSIVE MODE;
DO $escrow_supplement$
DECLARE before_money jsonb; after_money jsonb; result jsonb; prior jsonb;
BEGIN
 IF current_database()<>{L(t['database'])} OR current_user<>{L(t['role'])} OR session_user<>{L(t['login'])}
 OR (SELECT oid FROM pg_database WHERE datname=current_database())<>{int(t['database_oid'])}
 OR (pg_control_system()).system_identifier::text<>{L(t['system_identifier'])}
 OR current_setting('lmm.migration_instance_nonce')<>{L(t['nonce'])}
 THEN RAISE EXCEPTION 'escrow physical target mismatch'; END IF;
 IF NOT EXISTS(SELECT 1 FROM core_migration.import_runs WHERE migration_id={L(request['migration_id'])} AND source_archive_sha256={L(request['source_archive_sha256'])} AND mapping_sha256={L(request['mapping_sha256'])} AND manifest->>'scope'={L(W.SCOPE)} AND manifest->'partial'='true'::jsonb AND manifest->'full_financial_import_complete'='false'::jsonb)
 THEN RAISE EXCEPTION 'actual partial wallet anchor missing'; END IF;
 SELECT manifest INTO prior FROM core_migration.import_runs WHERE migration_id={L(m['migration_id'])};
 IF FOUND AND prior IS DISTINCT FROM {L(manifest)}::jsonb THEN RAISE EXCEPTION 'escrow supplement replay mismatch'; END IF;
 SELECT ({balances}) INTO before_money;
 result:=core_billing.import_legacy_escrow_batch({L(request)}::jsonb);
 SELECT ({balances}) INTO after_money;
 IF before_money IS DISTINCT FROM after_money THEN RAISE EXCEPTION 'escrow changed spendable wallet debt or affiliate'; END IF;
 {checks}
 IF NOT EXISTS(SELECT 1 FROM core_billing.legacy_escrow_batch_runs WHERE migration_id={L(request['migration_id'])} AND request={L(request)}::jsonb)
 OR (SELECT count(*) FROM core_billing.legacy_escrow_projects WHERE migration_id={L(request['migration_id'])})<>{len(request['projects'])}
 OR EXISTS(SELECT 1 FROM core_billing.legacy_escrow_projects p JOIN core_billing.ledger_entries e ON e.journal_id=p.opening_journal_id WHERE p.migration_id={L(request['migration_id'])} GROUP BY p.opening_journal_id HAVING sum(e.delta_units::numeric)<>0)
 THEN RAISE EXCEPTION 'escrow batch readback or conservation mismatch'; END IF;
 INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest) VALUES({L(m['migration_id'])},{L(m['source_archive_sha256'])},{L(W.digest(m))},{L(manifest)}::jsonb) ON CONFLICT DO NOTHING;
END $escrow_supplement$;
COMMIT;
SELECT jsonb_build_object('status','COMMITTED_ESCROW_RIGHTS_WALLET_UNCHANGED','projects',{len(request['projects'])},'claims',{sum(len(p['claims']) for p in request['projects'])},'full_financial_import_complete',false);
"""


def main():
    os.umask(0o077)
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--apply', action='store_true')
    for name in ['candidate', 'wallet-material', 'unit-evidence', 'policy-evidence', 'material']:
        p.add_argument('--'+name, type=Path)
    p.add_argument('--migration-id'); p.add_argument('--output', type=Path, required=True)
    a=p.parse_args(); a.output.mkdir(mode=0o700)
    outcome={'target_write_attempted': False, 'automatic_retry': False, 'actual_exit': 1}
    try:
        if not a.apply:
            m=prepare(W.load(a.candidate),W.load(a.wallet_material),W.load(a.unit_evidence),W.load(a.policy_evidence),a.migration_id)
            W.save(a.output,'material.private.json',m);W.save(a.output,'escrow.private.sql',build_sql(m))
            outcome.update(status='PREPARED_NOT_APPLIED',actual_exit=0)
        else:
            m=W.load(a.material);app='lmm-escrow-supplement-'+uuid.uuid4().hex
            W.save(a.output,'attempt.private.json',{'application_name':app,'migration_id':m['migration_id'],'material_sha256':W.digest(m),'started_epoch':time.time(),'target_has_previously_started':True})
            outcome['target_write_attempted']=True
            r=W.psql(m['connection'],build_sql(m),a.output,'apply',appname=app)
            if r.returncode==0:outcome.update(actual_exit=0,status='COMMITTED_ESCROW_RIGHTS_WALLET_UNCHANGED')
            else:outcome.update(status='OUTCOME_UNCONFIRMED',psql_exit=r.returncode)
    except Exception as e:
        outcome.update(status='OUTCOME_UNCONFIRMED' if outcome['target_write_attempted'] else 'REFUSED_BEFORE_WRITE',error=str(e))
    finally:W.save(a.output,'result.private.json',outcome)
    print(W.canonical(outcome));return outcome['actual_exit']


if __name__=='__main__':sys.exit(main())
