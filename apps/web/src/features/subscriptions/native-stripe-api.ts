/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

import {
  CommerceFailure,
  currentCommerceScope,
  nativeId,
  operationKey,
  type CommerceScope,
} from '@/features/commerce/native-client'
import { api } from '@/lib/api'

import { planSchema } from './native-api'
import {
  encodedPlan,
  safeCheckoutURL,
  subscriptionPaymentRequest,
} from './native-epay-api'

const decimalId = z
  .string()
  .regex(/^[1-9][0-9]{0,18}$/)
  .pipe(z.string().refine((v) => BigInt(v) <= 9223372036854775807n))
const safeId = z.union([nativeId, decimalId.transform(Number).pipe(nativeId)])
const orderId = z.string().regex(/^sub_[a-f0-9]{64}$/)
const timestamp = z
  .string()
  .regex(/^[1-9][0-9]{0,15}$/)
  .refine((v) => Number.isSafeInteger(Number(v)))
const optionalTime = timestamp.nullish().transform((v) => v ?? null)
const amount = decimalId
export const stripePriceSchema = z
  .object({
    plan_id: safeId,
    plan_revision: safeId,
    channel_id: safeId,
    currency: z.literal('USD'),
    amount_minor: amount,
    provider_price_id: z.string().regex(/^price_[A-Za-z0-9_]+$/),
    interval: z.enum(['day', 'week', 'month', 'year']),
    interval_count: safeId.pipe(z.number().max(365)),
    success_url: z
      .string()
      .url()
      .refine((v) => {
        try {
          safeCheckoutURL(v)
          return true
        } catch {
          return false
        }
      }),
    cancel_url: z
      .string()
      .url()
      .refine((v) => {
        try {
          safeCheckoutURL(v)
          return true
        } catch {
          return false
        }
      }),
  })
  .strict()
export type StripePrice = z.infer<typeof stripePriceSchema>
const orderSchema = z.object({
  order_id: orderId,
  account_id: decimalId,
  plan_id: decimalId,
  plan_revision: decimalId,
  payment_channel_id: decimalId,
  currency: z.literal('USD'),
  amount_minor: amount,
  expires_at_unix: timestamp,
  confirmed_at_unix: optionalTime,
  canceled_at_unix: optionalTime,
  proof_id: z.string().max(256),
  subscription_id: decimalId.nullish().transform((v) => v ?? null),
  granted_at_unix: optionalTime,
  ends_at_unix: optionalTime,
  plan_json: encodedPlan,
})
export const stripeOrderSchema = z
  .object({
    order: orderSchema,
    price: stripePriceSchema,
    merchant: z.string().min(1).max(256),
    environment: z.enum(['live', 'test']),
    provider_subscription_id: z.string().max(256),
    provider_customer_id: z.string().max(256),
    current_period_start: z.string().regex(/^[0-9]+$/),
    current_period_end: z.string().regex(/^[0-9]+$/),
    state: z.string().min(1).max(64),
    latest_grant_id: decimalId.nullish().transform((v) => v ?? null),
  })
  .superRefine((value, ctx) => {
    const o = value.order
    if (
      o.plan_id !== String(value.price.plan_id) ||
      o.plan_revision !== String(value.price.plan_revision) ||
      o.payment_channel_id !== String(value.price.channel_id) ||
      o.currency !== value.price.currency ||
      o.amount_minor !== value.price.amount_minor ||
      o.plan_id !== String(o.plan_json.id) ||
      o.plan_revision !== String(o.plan_json.revision)
    ) {
      ctx.addIssue({
        code: 'custom',
        message: 'Stripe order identity mismatch',
      })
    }
  })
export type StripeOrder = z.infer<typeof stripeOrderSchema>
export const stripeCheckoutSchema = z
  .object({
    id: z.string().min(1).max(256),
    url: z
      .string()
      .max(32768)
      .refine((v) => {
        try {
          safeCheckoutURL(v)
          return true
        } catch {
          return false
        }
      }),
    expires_at: timestamp,
  })
  .strict()
export const stripeViewSchema = z
  .object({
    order: stripeOrderSchema,
    checkout: stripeCheckoutSchema.nullable(),
  })
  .strict()
export type StripeView = z.infer<typeof stripeViewSchema>
export function freezeStripeOrder(value: StripeOrder) {
  return {
    account_id: value.order.account_id,
    plan: value.order.plan_json,
    price: value.price,
    merchant: value.merchant,
    environment: value.environment,
  }
}
const frozenSchema = z
  .object({
    account_id: decimalId,
    plan: planSchema,
    price: stripePriceSchema,
    merchant: z.string().min(1),
    environment: z.enum(['live', 'test']),
  })
  .strict()
export const stripePendingSchema = z
  .object({
    provider: z.literal('stripe'),
    operation_key: operationKey,
    plan_id: nativeId,
    channel_id: nativeId,
    order_id: orderId.optional(),
    frozen: frozenSchema.optional(),
    confirm_attempted: z.boolean(),
    cancel_attempted: z.boolean().optional(),
  })
  .strict()
  .superRefine((p, ctx) => {
    if (
      !!p.order_id !== !!p.frozen ||
      (p.confirm_attempted && !p.order_id) ||
      (p.frozen &&
        (p.frozen.price.plan_id !== p.plan_id ||
          p.frozen.price.channel_id !== p.channel_id))
    ) {
      ctx.addIssue({
        code: 'custom',
        message: 'Invalid Stripe recovery record',
      })
    }
  })
export type StripePending = z.infer<typeof stripePendingSchema>
const base = '/api/user/payments/subscriptions/stripe/orders'
function owned(
  scope: CommerceScope,
  pending: StripePending,
  value: StripeOrder
) {
  if (!currentCommerceScope(scope)) throw new CommerceFailure('SESSION_CHANGED')
  if (
    value.price.plan_id !== pending.plan_id ||
    value.price.channel_id !== pending.channel_id ||
    (pending.order_id && pending.order_id !== value.order.order_id) ||
    (pending.frozen &&
      JSON.stringify(pending.frozen) !==
        JSON.stringify(freezeStripeOrder(value)))
  ) {
    throw new CommerceFailure('FROZEN_ORDER_MISMATCH')
  }
  return value
}
export async function listStripePrices(scope: CommerceScope, planId: number) {
  nativeId.parse(planId)
  const response = await subscriptionPaymentRequest(
    scope,
    'get',
    `/core/v1/subscriptions/plans/${planId}/stripe-prices`,
    z.object({ prices: z.array(stripePriceSchema) })
  )
  if (response.prices.some((p) => p.plan_id !== planId)) {
    throw new CommerceFailure('PRICE_MISMATCH')
  }
  return response.prices
}
export async function prepareStripeOrder(
  scope: CommerceScope,
  pending: StripePending
) {
  stripePendingSchema.parse(pending)
  return owned(
    scope,
    pending,
    await subscriptionPaymentRequest(
      scope,
      'post',
      base,
      stripeOrderSchema,
      {
        plan_id: String(pending.plan_id),
        channel_id: String(pending.channel_id),
      },
      pending.operation_key
    )
  )
}
export async function readStripeOrder(
  scope: CommerceScope,
  pending: StripePending
) {
  if (!pending.order_id) throw new CommerceFailure('MISSING_ORDER')
  const view = await subscriptionPaymentRequest(
    scope,
    'get',
    `${base}/${orderId.parse(pending.order_id)}`,
    stripeViewSchema
  )
  owned(scope, pending, view.order)
  return view
}
export async function confirmStripeOrder(
  scope: CommerceScope,
  pending: StripePending
) {
  if (!pending.order_id) throw new CommerceFailure('MISSING_ORDER')
  const view = await subscriptionPaymentRequest(
    scope,
    'post',
    `${base}/${orderId.parse(pending.order_id)}/confirm`,
    stripeViewSchema
  )
  owned(scope, pending, view.order)
  return view
}
export function navigateStripeCheckout(scope: CommerceScope, view: StripeView) {
  if (!currentCommerceScope(scope)) throw new CommerceFailure('SESSION_CHANGED')
  if (
    !view.checkout ||
    view.order.latest_grant_id ||
    [
      'canceled',
      'incomplete_expired',
      'expired',
      'unpaid',
      'past_due',
      'paused',
    ].includes(view.order.state) ||
    !view.order.order.confirmed_at_unix ||
    view.order.order.proof_id ||
    view.order.order.subscription_id ||
    Number(view.checkout.expires_at) <= Date.now() / 1000
  ) {
    throw new CommerceFailure('INVALID_CHECKOUT')
  }
  const url = safeCheckoutURL(view.checkout.url)
  if (url.hostname !== 'checkout.stripe.com') {
    throw new CommerceFailure('INVALID_CHECKOUT')
  }
  window.location.assign(url.href)
}

/** A 202 acknowledges the request, never a verified subscription-state change. */
export async function cancelStripeRenewal(
  scope: CommerceScope,
  pending: StripePending
) {
  const fresh = await readStripeOrder(scope, pending)
  if (!fresh.order.provider_subscription_id) {
    throw new CommerceFailure('MISSING_SUBSCRIPTION')
  }
  if (!currentCommerceScope(scope)) throw new CommerceFailure('SESSION_CHANGED')
  try {
    const response = await api.request({
      method: 'post',
      url: `${base}/${orderId.parse(pending.order_id)}/cancel-renewal`,
      authScope: scope,
      disableDuplicate: true,
      skipAuthRefresh: true,
      skipBusinessError: true,
      skipErrorHandler: true,
      timeout: 15000,
    })
    if (!currentCommerceScope(scope)) {
      throw new CommerceFailure('SESSION_CHANGED')
    }
    if (response.status !== 202) throw new CommerceFailure('RESULT_UNKNOWN')
  } catch (error) {
    if (!currentCommerceScope(scope)) {
      throw new CommerceFailure('SESSION_CHANGED')
    }
    if (error instanceof CommerceFailure) throw error
    throw new CommerceFailure('RESULT_UNKNOWN')
  }
}
