#!/usr/bin/env python3
"""Emit read-only source export SQL; never connects to the frozen source cluster.

Run its stdout with psql -X -qAt -v ON_ERROR_STOP=1, redirect to a private file.
Includes full raw referral rows/audits and minimal identity/payment eligibility facts.
"""
import argparse
import re


def export_sql(schema='public'):
    if not re.fullmatch(r'[a-z_][a-z0-9_]*', schema):
        raise ValueError('invalid schema')
    return f'''BEGIN READ ONLY ISOLATION LEVEL REPEATABLE READ;
SET LOCAL search_path={schema},pg_catalog;
SELECT json_build_object(
 'users',(SELECT COALESCE(json_agg(t),'[]') FROM
  (SELECT id,inviter_id,referral_first_top_up_id,stripe_customer,email,status,aff_history FROM users ORDER BY id) t),
 'rewards',(SELECT COALESCE(json_agg(t),'[]') FROM (SELECT * FROM referral_rewards ORDER BY id) t),
 'ledger',(SELECT COALESCE(json_agg(t),'[]') FROM (SELECT * FROM referral_ledger_entries ORDER BY id) t),
 'referral_rebases',(SELECT COALESCE(json_agg(t),'[]') FROM (SELECT * FROM wallet_referral_credit_rebases ORDER BY reward_id) t),
 'wallet_rebases',(SELECT COALESCE(json_agg(t),'[]') FROM (SELECT * FROM wallet_credit_rebases ORDER BY migration_id) t),
 'payments',(SELECT COALESCE(json_agg(t),'[]') FROM (SELECT id,user_id,status,payment_method,payment_provider,
 settlement_currency,settled_amount_micros,expected_amount_micros,credited_quota,money,referral_excluded
 FROM top_ups ORDER BY id) t),
 'options',(SELECT COALESCE(json_object_agg(key,value),'{{}}') FROM options
 WHERE key LIKE 'Referral%' OR key IN ('QuotaForInviter','QuotaPerUnit','payment_setting.compliance_confirmed','payment_setting.compliance_terms_version')));
COMMIT;
'''


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--schema', default='public')
    print(export_sql(p.parse_args().schema))
