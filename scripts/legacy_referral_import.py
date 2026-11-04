#!/usr/bin/env python3
"""Plan owner-only import of complete raw facts and explicitly reviewed pending credit terms.

No economic units inferred. Terminal/revoked rows are retained raw, not replayed.
Review JSON binds source document hash and archive/migration, lists each earned
pending reward with current-credit terms, and ALL users' current affiliate history
and prior_cash facts. Raw policy remains the cash-floor authority. Empty/null policy
requires an explicit resolved snapshot. Rebase-affected pending contracts reject.
Use legacy_referral_export.py to ask the source owner for a consistent read-only export.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import re
from pathlib import Path

MAX = 9007199254740991
TERMS = ('registration_credit', 'reward_credit', 'max_reward_credit',
         'min_top_up_credit', 'max_penalty_credit')


def digest(v):
    return hashlib.sha256(json.dumps(v, sort_keys=True, separators=(',', ':'),
                                     ensure_ascii=False).encode()).hexdigest()


def lit(v):
    if v is None:
        return 'NULL'
    if isinstance(v, bool):
        return 'TRUE' if v else 'FALSE'
    if isinstance(v, int):
        return str(v)
    return "'" + str(v).replace("'", "''") + "'"


def jlit(v):
    return lit(json.dumps(v, ensure_ascii=False)) + '::jsonb'


def natural(v):
    if type(v) is not int or not 0 <= v <= MAX:
        raise ValueError('unknown or out-of-range credit units')
    return v


def plan(doc, review, archive, migration):
    if not re.fullmatch('[a-f0-9]{64}', archive) or not re.fullmatch('[A-Za-z0-9._:/-]{1,128}', migration):
        raise ValueError('invalid archive/migration identity')
    raw_doc = doc
    if 'ledger_entries' in doc:
        doc = {**doc, 'ledger': doc['ledger_entries'],
               'referral_rebases': doc['referral_credit_rebases'],
               'wallet_rebases': doc['wallet_credit_rebases']}
    if review['document_sha256'] != digest(raw_doc) or review['archive_sha256'] != archive or review['migration_id'] != migration:
        raise ValueError('review/source binding mismatch')
    if raw_doc.get('source_archive_sha256', archive) != archive:
        raise ValueError('archive identity mismatch')
    if review.get('unit') != 'credit_500k_usd' or not review.get('unit_evidence'):
        raise ValueError('reviewed unit evidence required')
    if any(type(u['id']) is not int or not 0 < u['id'] <= MAX for u in doc['users']):
        raise ValueError('invalid source user identity')
    users = {u['id']: u for u in doc['users']}
    rewards = {r['id']: r for r in doc['rewards']}
    if len(users) != len(doc['users']) or len(rewards) != len(doc['rewards']):
        raise ValueError('duplicate source identity')
    if any(e['reward_id'] not in rewards for e in doc['ledger']):
        raise ValueError('orphan source ledger')
    pending = {r['id']: r for r in rewards.values()
               if r['status'] == 'earned' and r['stage'] == 'awaiting_first_top_up'}
    terms = {r['id']: r for r in review['pending']}
    facts = {u['id']: u for u in review['users']}
    if set(terms) != set(pending) or set(facts) != set(users):
        raise ValueError('review must cover complete pending/user cohort')
    for audit in doc['wallet_rebases']:
        audit_plan = audit['plan']
        if isinstance(audit_plan, str):
            audit_plan = json.loads(audit_plan)
        affected = set(audit_plan.get('user_ids') or [])
        if audit_plan.get('include_affiliate') and any(r['inviter_id'] in affected for r in pending.values()):
            bases = audit_plan.get('referral_bases')
            if bases is None or any(b['reward_id'] in pending for b in bases):
                raise ValueError('pending referral rebase plan requires separate economic review')
    rebased = {b['reward_id'] for b in doc['referral_rebases']}
    if rebased & pending.keys():
        raise ValueError('pending referral rebase requires separate economic review')
    statements = ["BEGIN ISOLATION LEVEL SERIALIZABLE; SET LOCAL synchronous_commit=on;",
        "LOCK TABLE core_payments.intents,core_payments.legacy_applications IN SHARE MODE;",
        "DO $$ BEGIN IF current_user <> pg_get_userbyid((SELECT nspowner FROM pg_namespace WHERE nspname='core_referrals')) THEN RAISE EXCEPTION 'offline schema owner required'; END IF; IF EXISTS(SELECT FROM core_referrals.source) THEN RAISE EXCEPTION 'referral cohort already imported'; END IF; END $$;",
        f'INSERT INTO core_referrals.source VALUES({lit(archive)},{lit(migration)},{jlit(raw_doc)},{jlit(review)},DEFAULT);']
    for uid, u in users.items():
        f = facts[uid]
        if type(f['prior_cash']) is not bool or not f.get('evidence'):
            raise ValueError('reviewed prior cash evidence required')
        inviter = u['inviter_id'] or None
        # Opening relationship is checked for every imported source identity.
        statements.append(f"DO $$ BEGIN IF NOT EXISTS(SELECT FROM core_billing.opening_imports o JOIN core_identity.users u ON u.personal_account_id=o.account_id JOIN core_billing.opening_import_runs r USING(migration_id) WHERE u.id={lit(uid)} AND o.source_entity_key IN ({lit('user:'+str(uid))},{lit('legacy.users/'+str(uid))}) AND o.migration_id={lit(migration)} AND r.source_archive_sha256={lit(archive)}) THEN RAISE EXCEPTION 'referral opening binding mismatch'; END IF; END $$;")
        statements.append('INSERT INTO core_referrals.bindings VALUES('+','.join(map(lit, [uid, inviter, u['referral_first_top_up_id'], u.get('stripe_customer') or '', f['prior_cash'], archive, uid]))+');')
        statements.append(f'INSERT INTO core_referrals.history VALUES({lit(uid)},{natural(f["affiliate_history_credit"])});')
    for rid, r in pending.items():
        t = terms[rid]
        if r['invitee_id'] not in users or r['inviter_id'] not in users or users[r['invitee_id']]['inviter_id'] != r['inviter_id'] or r['top_up_id'] is not None:
            raise ValueError('pending binding mismatch')
        raw = r['policy_snapshot']
        policy = json.loads(raw) if raw else t.get('resolved_policy')
        if not isinstance(policy, dict) or not policy.get('min_top_up_amounts'):
            raise ValueError('missing policy must be explicitly resolved')
        expected = {'registration_credit': r['quota'], 'reward_credit': policy['reward_quota'],
                    'max_reward_credit': policy['max_reward_quota'],
                    'min_top_up_credit': policy['min_top_up_quota'],
                    'max_penalty_credit': r['max_penalty_quota']}
        if any(t[k] != expected[k] for k in TERMS) or r['quota'] != r['registration_quota']:
            raise ValueError('only evidenced unchanged pending deltas are supported')
        if t.get('source_policy_sha256') != digest(raw) or not t.get('evidence'):
            raise ValueError('policy/units evidence missing')
        # Source counters cannot be replayed. These are reviewed equivalents only.
        if r['revoked_quota'] != 0 or r['penalty_quota'] != 0:
            raise ValueError('earned pending has outstanding revocation counters')
        percent = natural(r['penalty_percent'])
        if percent > 100:
            raise ValueError('invalid source penalty percent')
        statements.append(f"DO $$ BEGIN IF EXISTS(SELECT FROM core_payments.intents WHERE created_by_user_id={lit(r['invitee_id'])} AND credit_journal_id IS NOT NULL) OR EXISTS(SELECT FROM core_payments.legacy_applications a JOIN core_payments.legacy_bases b ON b.id=a.basis_id WHERE b.user_id={lit(r['invitee_id'])} AND a.journal_id IS NOT NULL) THEN RAISE EXCEPTION 'referral import must precede target settlement'; END IF; END $$;")
        values = [lit(rid), lit(r['invitee_id']), lit(r['inviter_id']), jlit(policy)]
        values += [str(natural(t[k])) for k in TERMS]
        values += [str(percent), lit('awaiting_first_top_up'), lit('earned'), lit(''),
                   str(natural(t['registration_credit'])), lit(archive)]
        statements.append('INSERT INTO core_referrals.contracts(id,invitee,inviter,policy,'+','.join(TERMS)+',penalty_percent,stage,status,reason,quota,archive_sha256) VALUES('+','.join(values)+');')
    statements.append('COMMIT;')
    return '\n'.join(statements)+'\n'


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--source', type=Path, required=True)
    p.add_argument('--review', type=Path, required=True)
    p.add_argument('--archive-sha256', required=True)
    p.add_argument('--migration-id', required=True)
    p.add_argument('--output', type=Path, required=True)
    a = p.parse_args()
    import os
    sql = plan(json.loads(a.source.read_text()), json.loads(a.review.read_text()), a.archive_sha256, a.migration_id)
    with os.fdopen(os.open(a.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), 'w') as f:
        f.write(sql)
    print(json.dumps({'plan_sha256': hashlib.sha256(sql.encode()).hexdigest(), 'applied': False}))
