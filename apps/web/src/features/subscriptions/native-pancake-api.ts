/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

import {
  CommerceFailure,
  currentCommerceScope,
  nativeId,
  operationKey,
  type CommerceScope,
} from '@/features/commerce/native-client'

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
export const pancakePriceSchema = z
  .object({
    plan_id: safeId,
    plan_revision: safeId,
    channel_id: safeId,
    currency: z.enum(['USD', 'CNY']),
    amount_minor: amount,
    store_id: z.string().min(1).max(256),
    product_id: z.string().min(1).max(256),
    product_type: z.enum(['one_time', 'subscription']),
    billing_period: z.string().max(32),
    provider_plan_reference: z.string().max(256),
    return_url: z.string().url(),
    checkout_region: z.string().max(64),
    checkout_language: z.string().max(64),
    tax_category: z.string().max(64),
  })
  .strict()
export type PancakePrice = z.infer<typeof pancakePriceSchema>
const orderSchema = z.object({
  order_id: orderId,
  account_id: decimalId,
  plan_id: decimalId,
  plan_revision: decimalId,
  payment_channel_id: decimalId,
  currency: z.enum(['USD', 'CNY']),
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
export const pancakeOrderSchema = z
  .object({
    order: orderSchema,
    price: pancakePriceSchema,
    merchant: z.string().min(1).max(256),
    environment: z.enum(['live', 'test']),
    provider_reference: z.string().min(1).max(256),
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
        message: 'Pancake order identity mismatch',
      })
    }
  })
export type PancakeOrder = z.infer<typeof pancakeOrderSchema>
export const pancakeCheckoutSchema = z
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
export const pancakeViewSchema = z
  .object({
    order: pancakeOrderSchema,
    checkout: pancakeCheckoutSchema.nullable(),
  })
  .strict()
export type PancakeView = z.infer<typeof pancakeViewSchema>
export function freezePancakeOrder(value: PancakeOrder) {
  return {
    account_id: value.order.account_id,
    plan: value.order.plan_json,
    price: value.price,
    merchant: value.merchant,
    environment: value.environment,
    provider_reference: value.provider_reference,
  }
}
const frozenSchema = z
  .object({
    account_id: decimalId,
    plan: planSchema,
    price: pancakePriceSchema,
    merchant: z.string().min(1),
    environment: z.enum(['live', 'test']),
    provider_reference: z.string().min(1),
  })
  .strict()
export const pancakePendingSchema = z
  .object({
    provider: z.literal('waffo_pancake'),
    operation_key: operationKey,
    plan_id: nativeId,
    channel_id: nativeId,
    order_id: orderId.optional(),
    frozen: frozenSchema.optional(),
    confirm_attempted: z.boolean(),
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
        message: 'Invalid Pancake recovery record',
      })
    }
  })
export type PancakePending = z.infer<typeof pancakePendingSchema>
const base = '/api/user/payments/subscriptions/pancake/orders'
function owned(
  scope: CommerceScope,
  pending: PancakePending,
  value: PancakeOrder
) {
  if (!currentCommerceScope(scope)) throw new CommerceFailure('SESSION_CHANGED')
  if (
    value.price.plan_id !== pending.plan_id ||
    value.price.channel_id !== pending.channel_id ||
    (pending.order_id && pending.order_id !== value.order.order_id) ||
    (pending.frozen &&
      JSON.stringify(pending.frozen) !==
        JSON.stringify(freezePancakeOrder(value)))
  ) {
    throw new CommerceFailure('FROZEN_ORDER_MISMATCH')
  }
  return value
}
export async function listPancakePrices(scope: CommerceScope, planId: number) {
  nativeId.parse(planId)
  const response = await subscriptionPaymentRequest(
    scope,
    'get',
    `/core/v1/subscriptions/plans/${planId}/pancake-prices`,
    z.object({ prices: z.array(pancakePriceSchema) })
  )
  if (response.prices.some((p) => p.plan_id !== planId))
    throw new CommerceFailure('PRICE_MISMATCH')
  return response.prices
}
export async function preparePancakeOrder(
  scope: CommerceScope,
  pending: PancakePending
) {
  pancakePendingSchema.parse(pending)
  return owned(
    scope,
    pending,
    await subscriptionPaymentRequest(
      scope,
      'post',
      base,
      pancakeOrderSchema,
      {
        plan_id: String(pending.plan_id),
        channel_id: String(pending.channel_id),
      },
      pending.operation_key
    )
  )
}
export async function readPancakeOrder(
  scope: CommerceScope,
  pending: PancakePending
) {
  if (!pending.order_id) throw new CommerceFailure('MISSING_ORDER')
  const view = await subscriptionPaymentRequest(
    scope,
    'get',
    `${base}/${orderId.parse(pending.order_id)}`,
    pancakeViewSchema
  )
  owned(scope, pending, view.order)
  return view
}
export async function confirmPancakeOrder(
  scope: CommerceScope,
  pending: PancakePending
) {
  if (!pending.order_id) throw new CommerceFailure('MISSING_ORDER')
  const view = await subscriptionPaymentRequest(
    scope,
    'post',
    `${base}/${orderId.parse(pending.order_id)}/confirm`,
    pancakeViewSchema
  )
  owned(scope, pending, view.order)
  return view
}
export function navigatePancakeCheckout(
  scope: CommerceScope,
  view: PancakeView
) {
  if (!currentCommerceScope(scope)) throw new CommerceFailure('SESSION_CHANGED')
  if (
    !view.checkout ||
    view.order.latest_grant_id ||
    view.order.state === 'canceled' ||
    !view.order.order.confirmed_at_unix ||
    view.order.order.proof_id ||
    view.order.order.subscription_id ||
    Number(view.checkout.expires_at) <= Date.now() / 1000
  ) {
    throw new CommerceFailure('INVALID_CHECKOUT')
  }
  window.location.assign(safeCheckoutURL(view.checkout.url).href)
}
