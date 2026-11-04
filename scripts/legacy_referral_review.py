#!/usr/bin/env python3
"""Derive explicit unchanged-delta review from the frozen source rules, not FX.

Requires a complete prior-cash supplement. Raw source is never copied to stdout.
Only the reviewer-specified private output is written, with exclusive create 0600.
No PG connections, no money effects, no changes to original source file.
"""
import argparse
import json
import os
from pathlib import Path
from legacy_referral_import import digest, natural, MAX


def frozen_review(doc, supplement, migration):
    if doc['format_version'] != 1 or doc['options'].get('QuotaPerUnit') != '500000':
        raise ValueError('source credit scale not verified')
    rebases = doc['wallet_credit_rebases']
    if len(rebases) != 1:
        raise ValueError('unreviewed wallet rebase shape')
    rebase = rebases[0]['plan']
    if isinstance(rebase, str): rebase = json.loads(rebase)
    if rebase['usd_credit_conversion'] != 500000 or rebase.get('referral_bases') is None:
        raise ValueError('source current credit conversion/baselines unavailable')
    pending = [r for r in doc['rewards'] if r['status']=='earned' and r['stage']=='awaiting_first_top_up']
    prior = {r['invitee_id']: natural(r['qualifying_prior_count']) for r in supplement['pending_prior_cash']}
    if set(prior) != {r['invitee_id'] for r in pending} or len(prior)!=len(supplement['pending_prior_cash']):
        raise ValueError('prior-cash supplement must cover complete pending cohort')
    bases = {b['reward_id'] for b in rebase['referral_bases']}
    direct = {b['reward_id'] for b in doc['referral_credit_rebases']}
    if any(r['id'] in bases|direct for r in pending):
        raise ValueError('cannot derive unchanged delta for a rebased pending contract')
    options = doc['options']
    def option(key, default):
        return natural(int(options.get(key,default)))
    fallback = {'reward_quota':option('QuotaForInviter','0'),
        'registration_reward_quota':option('ReferralRegistrationRewardQuota','0'),
        'min_top_up_quota':option('ReferralMinTopUpQuota','0'),
        'max_reward_quota':option('ReferralMaxRewardQuota','0'),
        'penalty_percent':option('ReferralPenaltyPercent','20'),
        'max_penalty_quota':option('ReferralMaxPenaltyQuota','0'),
        'min_top_up_amounts':json.loads(options.get('ReferralMinTopUpAmounts','{"USD":"10","CNY":"70"}'))}
    terms=[]
    for r in pending:
        if r['quota'] != r['registration_quota']:
            raise ValueError('pending quota not solely historical registration')
        policy=json.loads(r['policy_snapshot']) if r['policy_snapshot'] else fallback
        t={'id':r['id'],'source_policy_sha256':digest(r['policy_snapshot']),
            'evidence':'frozen referral_credit_rebase.go unrebasedReferralDeltaTx: no per-reward row; complete wallet plan has referral_bases, none names this pending reward; original delta retained, no FX scaling',
            'registration_credit':natural(r['quota']), 'reward_credit':natural(policy['reward_quota']),
            'max_reward_credit':natural(policy['max_reward_quota']), 'min_top_up_credit':natural(policy['min_top_up_quota']),
            'max_penalty_credit':natural(r['max_penalty_quota'])}
        if not r['policy_snapshot']: t['resolved_policy']=fallback
        terms.append(t)
    return {'document_sha256':digest(doc),'archive_sha256':doc['source_archive_sha256'],
        'migration_id':migration,'unit':'credit_500k_usd',
        'unit_evidence':'source QuotaPerUnit=500000; wallet audit usd_credit_conversion=500000; counters already in opening, pending delta uses frozen unrebasedReferralDeltaTx unchanged-delta rule',
        'prior_cash_sha256':digest(supplement),'pending':terms,
        'users':[{'id':u['id'],'prior_cash':prior.get(u['id'],0)>0,
            'affiliate_history_credit':natural(u['aff_history']),
            'evidence':'complete source prior-cash pending cohort; other bindings are archive only, not executable contracts'} for u in doc['users']]}


if __name__ == '__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--source',type=Path,required=True)
    p.add_argument('--prior-cash',type=Path,required=True)
    p.add_argument('--migration-id',required=True)
    p.add_argument('--output',type=Path,required=True)
    a=p.parse_args()
    review=frozen_review(json.loads(a.source.read_text()),json.loads(a.prior_cash.read_text()),a.migration_id)
    with os.fdopen(os.open(a.output,os.O_WRONLY|os.O_CREAT|os.O_EXCL,0o600),'w') as f:
        json.dump(review,f,ensure_ascii=False,sort_keys=True)
    print(json.dumps({'review_sha256':digest(review),'pending_count':len(review['pending']),'applied':False}))
