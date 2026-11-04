"""Synthetic import contract for real Store tests; never fabricates payment facts."""
import json
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[3] / 'scripts'))
from legacy_referral_import import plan, digest

policy = {'reward_quota': 1000000, 'registration_reward_quota': 100000,
          'min_top_up_amounts': {'USD': '10', 'CNY': '70'}, 'min_top_up_quota': 0,
          'max_reward_quota': 0, 'penalty_percent': 20, 'max_penalty_quota': 0}
raw = json.dumps(policy)
doc = {'users': [{'id': 101, 'inviter_id': 102, 'referral_first_top_up_id': 0, 'stripe_customer': ''},
                 {'id': 102, 'inviter_id': 0, 'referral_first_top_up_id': 0, 'stripe_customer': ''}],
       'rewards': [{'id': 1, 'invitee_id': 101, 'inviter_id': 102, 'top_up_id': None,
                    'registration_quota': 100000, 'quota': 100000, 'policy_snapshot': raw,
                    'status': 'earned', 'stage': 'awaiting_first_top_up', 'revoked_quota': 0,
                    'penalty_quota': 0, 'penalty_percent': 20, 'max_penalty_quota': 0}],
       'ledger': [{'reward_id': 1, 'kind': 'registration_reward', 'quota': 100000}],
       'referral_rebases': [], 'wallet_rebases': []}
review = {'document_sha256': digest(doc), 'archive_sha256': 'a'*64, 'migration_id': 'synthetic-referral',
          'unit': 'credit_500k_usd', 'unit_evidence': 'synthetic reviewed current units',
          'users': [{'id': u, 'prior_cash': False, 'affiliate_history_credit': 100000 if u==102 else 0,
                     'evidence': 'synthetic historical cohort'} for u in [101,102]],
          'pending': [{'id': 1, 'source_policy_sha256': digest(raw), 'evidence': 'synthetic unchanged units',
                       'registration_credit': 100000, 'reward_credit': 1000000, 'max_reward_credit': 0,
                       'min_top_up_credit': 0, 'max_penalty_credit': 0}]}
print(plan(doc, review, 'a'*64, 'synthetic-referral'))
