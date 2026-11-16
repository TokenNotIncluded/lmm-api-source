/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import type { NativeUser, NativeUserStatistics } from './contract'

/** Synthetic read data shared only by tests and the gated local persona preview. */
export function nativeUserStatisticsFixture(
  user: Pick<NativeUser, 'id' | 'personal_account_id' | 'wallet'>
): NativeUserStatistics {
  return {
    user_id: user.id,
    personal_account_id: user.personal_account_id,
    usage: {
      status: 'available',
      scope: 'actor_settled_and_complete_history',
      requests: '1234',
      tokens: '9876543',
      credits: '250000',
      model_count: '3',
      unit: 'credit_500k_usd',
      credits_per_usd: 500000,
      pricing_available: true,
      history_source: 'quota_aggregates',
      observed_at_unix_ms: 1790035200000,
    },
    historical_counters: {
      status: 'available',
      unit: 'legacy_raw_units',
      used_quota: '-17',
      request_count: '76',
      affiliate_rewards: '450000',
      observed_at_unix_ms: 1782864000000,
    },
    referrals: {
      invited_users: '12',
      invitation_scope: 'recorded_native_and_imported_bindings',
      affiliate_balance_units:
        user.wallet.status === 'available' ? '125000' : null,
      unit: 'core_credit',
      money_basis: 'unknown',
      reviewed_rewards: {
        scope: 'reviewed_history_and_later_settlements',
        granted_credits: '700000',
        unit: 'credit_500k_usd',
        credits_per_usd: 500000,
      },
    },
  }
}
