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
export const creemPriceSchema = z
  .object({
    plan_id: safeId,
    plan_revision: safeId,
    channel_id: safeId,
    currency: z.enum(['USD', 'CNY', 'EUR']),
    amount_minor: amount,
    provider_price_id: z.string().regex(/^prod_[A-Za-z0-9_]+$/),
    interval: z.enum(['month', 'year']),
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
  .refine(
    (p) =>
      (p.interval === 'month' && [1, 3, 6].includes(p.interval_count)) ||
      (p.interval === 'year' && p.interval_count === 1)
  )
export type CreemPrice = z.infer<typeof creemPriceSchema>
const orderSchema = z.object({
  order_id: orderId,
  account_id: decimalId,
  plan_id: decimalId,
  plan_revision: decimalId,
  payment_channel_id: decimalId,
  currency: z.enum(['USD', 'CNY', 'EUR']),
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
export const creemOrderSchema = z
  .object({
    order: orderSchema,
    price: creemPriceSchema,
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
        message: 'Creem order identity mismatch',
      })
    }
  })
export type CreemOrder = z.infer<typeof creemOrderSchema>
export const creemCheckoutSchema = z
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
export const creemViewSchema = z
  .object({
    order: creemOrderSchema,
    checkout: creemCheckoutSchema.nullable(),
  })
  .strict()
export type CreemView = z.infer<typeof creemViewSchema>
export function freezeCreemOrder(value: CreemOrder) {
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
    price: creemPriceSchema,
    merchant: z.string().min(1),
    environment: z.enum(['live', 'test']),
  })
  .strict()
export const creemPendingSchema = z
  .object({
    provider: z.literal('creem'),
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
        message: 'Invalid Creem recovery record',
      })
    }
  })
export type CreemPending = z.infer<typeof creemPendingSchema>
const base = '/api/user/payments/subscriptions/creem/orders'
function owned(scope: CommerceScope, pending: CreemPending, value: CreemOrder) {
  if (!currentCommerceScope(scope)) throw new CommerceFailure('SESSION_CHANGED')
  if (
    value.price.plan_id !== pending.plan_id ||
    value.price.channel_id !== pending.channel_id ||
    (pending.order_id && pending.order_id !== value.order.order_id) ||
    (pending.frozen &&
      JSON.stringify(pending.frozen) !==
        JSON.stringify(freezeCreemOrder(value)))
  ) {
    throw new CommerceFailure('FROZEN_ORDER_MISMATCH')
  }
  return value
}
export async function listCreemPrices(scope: CommerceScope, planId: number) {
  nativeId.parse(planId)
  const response = await subscriptionPaymentRequest(
    scope,
    'get',
    `/core/v1/subscriptions/plans/${planId}/creem-prices`,
    z.object({ prices: z.array(creemPriceSchema) })
  )
  if (response.prices.some((p) => p.plan_id !== planId)) {
    throw new CommerceFailure('PRICE_MISMATCH')
  }
  return response.prices
}
export async function prepareCreemOrder(
  scope: CommerceScope,
  pending: CreemPending
) {
  creemPendingSchema.parse(pending)
  return owned(
    scope,
    pending,
    await subscriptionPaymentRequest(
      scope,
      'post',
      base,
      creemOrderSchema,
      {
        plan_id: String(pending.plan_id),
        channel_id: String(pending.channel_id),
      },
      pending.operation_key
    )
  )
}
export async function readCreemOrder(
  scope: CommerceScope,
  pending: CreemPending
) {
  if (!pending.order_id) throw new CommerceFailure('MISSING_ORDER')
  const view = await subscriptionPaymentRequest(
    scope,
    'get',
    `${base}/${orderId.parse(pending.order_id)}`,
    creemViewSchema
  )
  owned(scope, pending, view.order)
  return view
}
export async function confirmCreemOrder(
  scope: CommerceScope,
  pending: CreemPending
) {
  if (!pending.order_id) throw new CommerceFailure('MISSING_ORDER')
  const view = await subscriptionPaymentRequest(
    scope,
    'post',
    `${base}/${orderId.parse(pending.order_id)}/confirm`,
    creemViewSchema
  )
  owned(scope, pending, view.order)
  return view
}
export function navigateCreemCheckout(scope: CommerceScope, view: CreemView) {
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
  if (
    ![
      'checkout.creem.io',
      'creem.io',
      'www.creem.io',
      'test-checkout.creem.io',
    ].includes(url.hostname)
  ) {
    throw new CommerceFailure('INVALID_CHECKOUT')
  }
  window.location.assign(url.href)
}

/** A 202 acknowledges the request, never a verified subscription-state change. */
export async function cancelCreemRenewal(
  scope: CommerceScope,
  pending: CreemPending
) {
  const fresh = await readCreemOrder(scope, pending)
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
