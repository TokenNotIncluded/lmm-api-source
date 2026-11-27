/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import type { NativePlan, NativePurchase } from '../native-api'
import type {
  PlanRecord,
  SelfSubscriptionData,
  UserSubscriptionRecord,
} from '../types'
import { SUBSCRIPTION_BALANCE_PAYMENT_METHOD } from './payment-methods'

function safeUnits(value: string): number {
  if (!/^(0|[1-9][0-9]*)$/.test(value)) {
    throw new Error('Invalid subscription credit amount')
  }
  const n = Number(value)
  if (!Number.isSafeInteger(n)) {
    throw new Error('Subscription credit amount exceeds the classic projection')
  }
  return n
}

function durationFields(
  plan: NativePlan
): Pick<
  PlanRecord['plan'],
  'duration_unit' | 'duration_value' | 'custom_seconds'
> {
  const months = plan.terms.duration_months ?? 0
  if (months > 0) {
    return { duration_unit: 'month', duration_value: months }
  }
  const seconds = plan.terms.duration_seconds
  if (seconds >= 86400 && seconds % 86400 === 0) {
    return { duration_unit: 'day', duration_value: seconds / 86400 }
  }
  if (seconds >= 3600 && seconds % 3600 === 0) {
    return { duration_unit: 'hour', duration_value: seconds / 3600 }
  }
  return {
    duration_unit: 'custom',
    duration_value: 1,
    custom_seconds: seconds,
  }
}

function resetFields(
  plan: NativePlan
): Pick<
  PlanRecord['plan'],
  'quota_reset_period' | 'quota_reset_custom_seconds'
> {
  switch (plan.terms.reset_period) {
    case 'day':
      return { quota_reset_period: 'daily' }
    case 'week':
      return { quota_reset_period: 'weekly' }
    case 'month':
    case 'anniversary_month':
      return { quota_reset_period: 'monthly' }
    case 'custom':
      return {
        quota_reset_period: 'custom',
        quota_reset_custom_seconds: plan.terms.reset_seconds,
      }
    case 'grant':
    default:
      return { quota_reset_period: 'never' }
  }
}

/** Present a Core plan in the classic wallet subscription card shape. */
export function nativePlanToRecord(plan: NativePlan): PlanRecord {
  const priceQuota = safeUnits(plan.terms.price_units)
  const totalQuota = safeUnits(plan.terms.limit_units)
  const methods = plan.terms.allow_balance_pay
    ? [SUBSCRIPTION_BALANCE_PAYMENT_METHOD]
    : []
  return {
    contract: 'core.subscriptions.v1',
    plan: {
      id: plan.id,
      title: plan.terms.title,
      price_amount: 0,
      // Not CNY/USD → classic card falls back to balance_price_quota display.
      currency: 'CREDIT',
      ...durationFields(plan),
      ...resetFields(plan),
      enabled: plan.terms.enabled && !plan.archived,
      archived: plan.archived,
      sort_order: plan.id,
      allow_balance_pay: plan.terms.allow_balance_pay,
      allow_wallet_overflow: true,
      max_purchase_per_user: plan.terms.max_purchases_per_user,
      total_amount: totalQuota,
    },
    payment_methods: methods,
    balance_price_quota: priceQuota,
  }
}

function nativePurchaseStatus(
  purchase: NativePurchase,
  checkedAt: number
): string {
  if (purchase.refund_journal_id != null) return 'cancelled'
  if (purchase.entitlement_active) return 'active'
  if (purchase.starts_at > checkedAt) return 'active'
  if (purchase.ends_at < checkedAt) return 'expired'
  return 'expired'
}

export function nativePurchaseToRecord(
  purchase: NativePurchase,
  userId: number,
  checkedAt = Date.now() / 1000
): UserSubscriptionRecord {
  const total = safeUnits(purchase.plan.terms.limit_units)
  const used = safeUnits(purchase.used_units)
  return {
    subscription: {
      id: purchase.subscription_id,
      user_id: userId,
      plan_id: purchase.plan.id,
      status: nativePurchaseStatus(purchase, checkedAt),
      start_time: purchase.starts_at,
      end_time: purchase.ends_at,
      amount_total: total,
      amount_used: used,
    },
  }
}

export function nativePurchasesToSelfData(
  purchases: NativePurchase[],
  userId: number,
  checkedAt = Date.now() / 1000
): SelfSubscriptionData {
  const all = purchases.map((purchase) =>
    nativePurchaseToRecord(purchase, userId, checkedAt)
  )
  const subscriptions = all.filter((item) => {
    const sub = item.subscription
    return (
      sub.status === 'active' &&
      sub.end_time >= checkedAt &&
      sub.start_time <= checkedAt
    )
  })
  return {
    contract: 'core.subscriptions.v1',
    billing_preference: 'wallet_first',
    subscriptions,
    all_subscriptions: all,
  }
}
