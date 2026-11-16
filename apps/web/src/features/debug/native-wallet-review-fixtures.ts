/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { AxiosError, type InternalAxiosRequestConfig } from 'axios'

import type {
  NativeCampaign,
  NativeReward,
} from '@/features/promotions/native-api'
import type {
  NativePlan,
  NativePurchase,
} from '@/features/subscriptions/native-api'
import { useAuthStore, type AuthUser } from '@/stores/auth-store'

const profiles: Record<
  number,
  {
    persona: string
    spendable: number
    reserved: number
    used: number
    requests: number
  }
> = {
  1001: { persona: 'l0', spendable: 0, reserved: 0, used: 0, requests: 0 },
  1002: {
    persona: 'l1',
    spendable: 500000,
    reserved: 25000,
    used: 125000,
    requests: 42,
  },
  1003: { persona: 'b', spendable: 0, reserved: 0, used: 0, requests: 3 },
  1004: {
    persona: 'e',
    spendable: 750000,
    reserved: 125000,
    used: 190000,
    requests: 88,
  },
  1005: {
    persona: 'f',
    spendable: 2000000,
    reserved: 125000,
    used: 340000,
    requests: 120,
  },
  1099: {
    persona: 'admin',
    spendable: 10000000,
    reserved: 125000,
    used: 2500000,
    requests: 390,
  },
}
const startsAt = 1793491200
const endsAt = 1796083200
const plans: NativePlan[] = [
  {
    id: 401,
    revision: 1,
    archived: false,
    terms: {
      title: 'Developer monthly',
      price_units: '250000',
      duration_seconds: 0,
      duration_months: 1,
      reset_period: 'month',
      reset_seconds: 0,
      limit_units: '500000',
      models: ['*'],
      groups: ['*'],
      max_purchases_per_user: 1,
      enabled: true,
      allow_balance_pay: true,
    },
    payment_methods: ['wallet'],
  },
  {
    id: 402,
    revision: 1,
    archived: false,
    terms: {
      title: 'Team monthly',
      price_units: '1250000',
      duration_seconds: 0,
      duration_months: 1,
      reset_period: 'month',
      reset_seconds: 0,
      limit_units: '3000000',
      models: ['*'],
      groups: ['*'],
      max_purchases_per_user: 1,
      enabled: true,
      allow_balance_pay: true,
    },
    payment_methods: ['wallet'],
  },
]
const campaigns: NativeCampaign[] = (['checkin', 'envelope'] as const).map(
  (kind, index) => ({
    id: String(index + 1).repeat(64),
    revision: 1,
    spec: {
      kind,
      title: kind === 'checkin' ? 'Daily check-in' : 'Community reward',
      award_units: '10000',
      budget_units: '1000000',
      max_claims: 100,
      max_claims_per_user: 1,
      starts_at: startsAt,
      ends_at: endsAt,
      enabled: true,
    },
    spent_units: '10000',
    reserved_units: '0',
    claims: 1,
    reserved_claims: 0,
  })
)

/** The development entry applies this only for console_review=1. */
export function nativeWalletReviewProfile(user: AuthUser): AuthUser {
  const profile = profiles[user.id]
  if (!profile) {
    return structuredClone(user)
  }
  return {
    ...structuredClone(user),
    financial_projection_available: true,
    usage_projection_available: true,
    wallet_unit: 'core_credit',
    wallet_money_basis: 'unknown',
    wallet_money_available: false,
    quota: profile.spendable,
    wallet_spendable_quota: profile.spendable,
    wallet_reserved_quota: profile.reserved,
    wallet_debt: 0,
    used_quota: profile.used,
    normalized_used_quota: profile.used,
    request_count: profile.requests,
    aff_quota: 0,
    aff_debt: 0,
    aff_history_quota: 0,
    aff_count: 0,
  }
}

function requestURL(config: InternalAxiosRequestConfig): URL | undefined {
  try {
    if (!config.url) {
      return undefined
    }
    const base = new URL(
      config.baseURL ?? window.location.origin,
      window.location.origin
    )
    const url = new URL(config.url, base)
    if (
      url.origin !== window.location.origin ||
      url.username ||
      url.password ||
      url.hash
    ) {
      return undefined
    }
    const params: unknown = config.params
    if (params instanceof URLSearchParams) {
      for (const [key, value] of params) {
        url.searchParams.append(key, value)
      }
    } else if (params && typeof params === 'object' && !Array.isArray(params)) {
      for (const [key, value] of Object.entries(params)) {
        if (value === undefined || value === null) {
          continue
        }
        if (typeof value !== 'number' && typeof value !== 'string') {
          return undefined
        }
        url.searchParams.append(key, String(value))
      }
    } else if (params !== undefined && params !== null) {
      return undefined
    }
    return url
  } catch {
    return undefined
  }
}

function transfers(user: AuthUser) {
  const profile = profiles[user.id]
  if (!profile?.reserved) {
    return []
  }
  return (['pending', 'claimed', 'cancelled'] as const).map((status, index) => {
    const id = user.id * 10 + 3 - index
    const quota = status === 'pending' ? profile.reserved : 25000
    const token = id.toString(16).padStart(64, '0')
    return {
      id,
      quota,
      status,
      created_at: startsAt + (3 - index) * 86400,
      claimed_at: status === 'claimed' ? startsAt + 4 * 86400 : 0,
      cancelled_at: status === 'cancelled' ? startsAt + 4 * 86400 : 0,
      request_key: `synthetic-transfer-${user.id}-${id}`,
      ...(status === 'pending'
        ? {
            token,
            share_url: `${window.location.origin}/transfer#${token}`,
          }
        : {}),
      claim_link_available: status === 'pending',
      recipient_details_available: false,
      public_credit_amount: String(quota),
      credit_unit_schema_version: 2,
      quota_unit: 'LEDGER_QUOTA',
      public_credit_unit: 'CREDIT',
      public_credits_per_usd_exact: '500000',
    }
  })
}

/**
 * Synthetic GET data for the loopback console review entry only.
 * No quote, confirmation, claim, cancellation or payment write is simulated.
 * Unknown paths fall through to the development adapter's explicit refusal.
 */
export function nativeWalletReviewFixture(
  config: InternalAxiosRequestConfig
): unknown {
  if ((config.method ?? 'get').toUpperCase() !== 'GET') {
    return undefined
  }
  const url = requestURL(config)
  const { user, session, accessToken } = useAuthStore.getState().auth
  const profile = user ? profiles[user.id] : undefined
  if (
    !url ||
    !user ||
    !profile ||
    !session ||
    session.sid !== `debug-persona-${profile.persona}` ||
    accessToken !== session.sid ||
    (config.authScope &&
      (config.authScope.userId !== user.id ||
        config.authScope.sessionId !== session.sid))
  ) {
    return undefined
  }

  if (url.pathname === '/api/wallet-transfer') {
    const beforeValues = url.searchParams.getAll('before')
    if (
      [...url.searchParams.keys()].some((key) => key !== 'before') ||
      beforeValues.length > 1
    ) {
      return undefined
    }
    const raw = beforeValues[0] ?? '0'
    const before = Number(raw)
    if (!/^(0|[1-9][0-9]*)$/.test(raw) || !Number.isSafeInteger(before)) {
      return undefined
    }
    return {
      success: true,
      message: '',
      data: transfers(user).filter((row) => !before || row.id < before),
    }
  }
  if (url.search) {
    return undefined
  }

  if (
    url.pathname === '/api/subscription/plans' ||
    url.pathname === '/api/subscription/self'
  ) {
    throw new AxiosError(
      'Native subscription review route',
      AxiosError.ERR_BAD_REQUEST,
      config,
      undefined,
      {
        config,
        status: 404,
        statusText: 'Not Found',
        headers: {},
        data: { error: { code: 'ROUTE_NOT_FOUND' } },
      }
    )
  }
  if (url.pathname === '/api/wallet/balance') {
    return {
      success: true,
      message: '',
      data: {
        schema_version: 2,
        currency_unit: 'CREDIT',
        credits_per_usd: 500000,
        public_credits_per_usd_exact: '500000',
        available_quota: profile.spendable,
        available_credits: profile.spendable,
        spendable_credits: profile.spendable,
        reserved_credits: profile.reserved,
        debt_credits: 0,
        affiliate_credits: 0,
        public_available_credits: String(profile.spendable),
      },
    }
  }
  if (url.pathname === '/core/v1/subscriptions/plans') {
    return {
      contract: 'core.subscriptions.v1',
      plans: structuredClone(plans),
      payment_methods: ['wallet'],
      automatic_renewal: false,
    }
  }
  if (url.pathname === '/core/v1/subscriptions/purchases') {
    const plan = plans[0]
    const purchases: NativePurchase[] =
      profile.spendable && plan
        ? [
            {
              id: user.id * 10 + 1,
              subscription_id: user.id * 10 + 2,
              plan: structuredClone(plan),
              starts_at: startsAt,
              ends_at: endsAt,
              used_units: '75000',
              reserved_units: '12500',
              entitlement_active: true,
              renewal_canceled: false,
              canceled_at: null,
              refund_journal_id: null,
            },
          ]
        : []
    return { contract: 'core.subscriptions.v1', purchases }
  }
  if (
    /^\/core\/v1\/subscriptions\/plans\/(401|402)\/(epay|pancake|stripe|creem)-prices$/.test(
      url.pathname
    )
  ) {
    return { prices: [] }
  }
  if (url.pathname === '/core/v1/promotions/campaigns') {
    return {
      contract: 'core.promotions.v1',
      campaigns: structuredClone(campaigns),
    }
  }
  if (url.pathname === '/core/v1/promotions/history') {
    const claims: NativeReward[] = profile.spendable
      ? [
          {
            id: user.id.toString(16).padStart(64, '0'),
            campaign_id: '1'.repeat(64),
            amount_units: '10000',
            journal_id: user.id * 10 + 4,
            claimed_at: startsAt + 86400,
          },
        ]
      : []
    return { contract: 'core.promotions.v1', claims }
  }
  if (url.pathname === '/api/user/payments/channels') {
    return {
      success: true,
      data: {
        available: true,
        pancake_portal_available: false,
        channels: [
          {
            id: '701',
            provider: 'stripe',
            environment: 'test',
            currency: 'USD',
          },
        ],
      },
    }
  }
  return undefined
}
