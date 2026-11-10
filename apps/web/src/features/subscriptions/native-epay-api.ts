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

const decimalId = z
  .string()
  .regex(/^[1-9][0-9]{0,18}$/)
  .refine((value) => BigInt(value) <= 9223372036854775807n)
const orderId = z.string().regex(/^sub_[a-f0-9]{64}$/)
const timestamp = z
  .string()
  .regex(/^[1-9][0-9]{0,15}$/)
  .refine((value) => Number.isSafeInteger(Number(value)))
const optionalTimestamp = timestamp
  .nullish()
  .transform((value) => value ?? null)
const minorAmount = z
  .string()
  .regex(/^[1-9][0-9]{0,18}$/)
  .refine((value) => BigInt(value) <= 9223372036854775807n)

export const epayPriceSchema = z.object({
  plan_id: nativeId,
  plan_revision: nativeId,
  channel_id: nativeId,
  currency: z.enum(['CNY', 'LDC']),
  amount_minor: minorAmount,
  channel_name: z.string().trim().min(1).max(160).optional(),
})
export type EPayPrice = z.infer<typeof epayPriceSchema>

export function safeCheckoutURL(value: string) {
  const url = new URL(value)
  if (url.protocol !== 'https:' || url.username || url.password || url.hash) {
    throw new CommerceFailure('INVALID_CHECKOUT')
  }
  return url
}
export const epayCheckoutSchema = z.object({
  intent_id: z.literal('0'),
  order_id: orderId,
  method: z.enum(['GET', 'POST']),
  checkout_url: z
    .string()
    .min(1)
    .max(32768)
    .refine((value) => {
      try {
        safeCheckoutURL(value)
        return true
      } catch {
        return false
      }
    }),
})
export type EPayCheckout = z.infer<typeof epayCheckoutSchema>

const encodedPlan = z
  .string()
  .min(1)
  .max(262144)
  .transform((value, ctx) => {
    try {
      // RPC bytes are base64, and JSON is UTF-8 (including non-ASCII plan titles).
      const bytes = Uint8Array.from(atob(value), (char) => char.charCodeAt(0))
      return planSchema.parse(
        JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes))
      )
    } catch {
      ctx.addIssue({ code: 'custom', message: 'Invalid frozen plan' })
      return z.NEVER
    }
  })
export const epayOrderSchema = z
  .object({
    order_id: orderId,
    account_id: decimalId,
    plan_id: decimalId,
    plan_revision: decimalId,
    payment_channel_id: decimalId,
    currency: z.enum(['CNY', 'LDC']),
    amount_minor: minorAmount,
    expires_at_unix: timestamp,
    confirmed_at_unix: optionalTimestamp,
    canceled_at_unix: optionalTimestamp,
    proof_id: z.string().max(256),
    subscription_id: decimalId.nullish().transform((value) => value ?? null),
    granted_at_unix: optionalTimestamp,
    ends_at_unix: optionalTimestamp,
    plan_json: encodedPlan,
    checkout: epayCheckoutSchema.nullish(),
  })
  .superRefine((order, ctx) => {
    if (
      String(order.plan_json.id) !== order.plan_id ||
      String(order.plan_json.revision) !== order.plan_revision ||
      (order.checkout && order.checkout.order_id !== order.order_id)
    ) {
      ctx.addIssue({ code: 'custom', message: 'Order identity mismatch' })
    }
  })
export type EPayOrder = z.infer<typeof epayOrderSchema>
const frozenTermsSchema = z
  .object({
    account_id: decimalId,
    plan_revision: decimalId,
    currency: z.enum(['CNY', 'LDC']),
    amount_minor: minorAmount,
    plan: planSchema,
  })
  .strict()
export function freezeEPayOrder(order: EPayOrder) {
  return {
    account_id: order.account_id,
    plan_revision: order.plan_revision,
    currency: order.currency,
    amount_minor: order.amount_minor,
    plan: order.plan_json,
  }
}
export const epayPendingSchema = z
  .object({
    operation_key: operationKey,
    plan_id: nativeId,
    channel_id: nativeId,
    order_id: orderId.optional(),
    confirm_attempted: z.boolean(),
    cancel_attempted: z.boolean().optional(),
    frozen: frozenTermsSchema.optional(),
    checkout: epayCheckoutSchema.optional(),
  })
  .strict()
  .superRefine((pending, ctx) => {
    if (
      !!pending.order_id !== !!pending.frozen ||
      ((pending.confirm_attempted || pending.cancel_attempted) &&
        !pending.order_id) ||
      (pending.checkout &&
        (pending.checkout.order_id !== pending.order_id ||
          !pending.confirm_attempted))
    ) {
      ctx.addIssue({ code: 'custom', message: 'Invalid recovery record' })
    }
  })
export type EPayPending = z.infer<typeof epayPendingSchema>
const base = '/api/user/payments/subscriptions/orders'

async function request<T>(
  scope: CommerceScope,
  method: 'get' | 'post',
  url: string,
  schema: z.ZodType<T>,
  data?: unknown,
  key?: string
): Promise<T> {
  if (!currentCommerceScope(scope)) throw new CommerceFailure('SESSION_CHANGED')
  try {
    const response = await api.request({
      method,
      url,
      data,
      headers: key ? { 'Idempotency-Key': operationKey.parse(key) } : undefined,
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
    const parsed = schema.safeParse(response.data)
    if (!parsed.success) throw new CommerceFailure('INVALID_RESPONSE')
    return parsed.data
  } catch (error) {
    if (!currentCommerceScope(scope)) {
      throw new CommerceFailure('SESSION_CHANGED')
    }
    if (error instanceof CommerceFailure) throw error
    // A failed response cannot prove that a prior financial dispatch did not commit.
    throw new CommerceFailure('RESULT_UNKNOWN')
  }
}
function ownedOrder(
  scope: CommerceScope,
  pending: EPayPending,
  order: EPayOrder
) {
  if (!currentCommerceScope(scope)) throw new CommerceFailure('SESSION_CHANGED')
  if (
    order.plan_id !== String(pending.plan_id) ||
    order.payment_channel_id !== String(pending.channel_id) ||
    (pending.order_id && pending.order_id !== order.order_id)
  ) {
    throw new CommerceFailure('ORDER_MISMATCH')
  }
  if (
    pending.frozen &&
    JSON.stringify(pending.frozen) !== JSON.stringify(freezeEPayOrder(order))
  ) {
    throw new CommerceFailure('FROZEN_ORDER_MISMATCH')
  }
  return order
}
export async function listEPayPrices(scope: CommerceScope, planId: number) {
  nativeId.parse(planId)
  const result = await request(
    scope,
    'get',
    `/core/v1/subscriptions/plans/${planId}/epay-prices`,
    z.object({ prices: z.array(epayPriceSchema) })
  )
  if (result.prices.some((price) => price.plan_id !== planId)) {
    throw new CommerceFailure('PRICE_MISMATCH')
  }
  return result.prices
}
export async function prepareEPayOrder(
  scope: CommerceScope,
  pending: EPayPending
) {
  epayPendingSchema.parse(pending)
  return ownedOrder(
    scope,
    pending,
    await request(
      scope,
      'post',
      base,
      epayOrderSchema,
      { plan_id: pending.plan_id, channel_id: pending.channel_id },
      pending.operation_key
    )
  )
}
export async function readEPayOrder(
  scope: CommerceScope,
  pending: EPayPending
) {
  if (!pending.order_id) throw new CommerceFailure('MISSING_ORDER')
  return ownedOrder(
    scope,
    pending,
    await request(
      scope,
      'get',
      `${base}/${orderId.parse(pending.order_id)}`,
      epayOrderSchema
    )
  )
}
export async function confirmEPayOrder(scope: CommerceScope, order: EPayOrder) {
  const result = await request(
    scope,
    'post',
    `${base}/${orderId.parse(order.order_id)}/confirm`,
    epayCheckoutSchema
  )
  if (result.order_id !== order.order_id) {
    throw new CommerceFailure('ORDER_MISMATCH')
  }
  return result
}
export async function cancelEPayOrder(
  scope: CommerceScope,
  pending: EPayPending
) {
  if (!pending.order_id) throw new CommerceFailure('MISSING_ORDER')
  const order = ownedOrder(
    scope,
    pending,
    await request(
      scope,
      'post',
      `${base}/${orderId.parse(pending.order_id)}/cancel`,
      epayOrderSchema
    )
  )
  if (!order.canceled_at_unix) throw new CommerceFailure('INVALID_RESPONSE')
  return order
}

/** Only called by an explicit user action, after a fresh owned-order read. */
export function navigateEPayCheckout(
  scope: CommerceScope,
  checkout: EPayCheckout
) {
  if (!currentCommerceScope(scope)) throw new CommerceFailure('SESSION_CHANGED')
  epayCheckoutSchema.parse(checkout)
  const url = safeCheckoutURL(checkout.checkout_url)
  if (checkout.method === 'GET') {
    window.location.assign(url.href)
    return
  }
  const form = document.createElement('form')
  form.method = 'POST'
  form.acceptCharset = 'UTF-8'
  form.action = `${url.origin}${url.pathname}`
  for (const [name, value] of url.searchParams) {
    const input = document.createElement('input')
    input.type = 'hidden'
    input.name = name
    input.value = value
    form.appendChild(input)
  }
  document.body.appendChild(form)
  try {
    // Named fields such as "submit" cannot replace the native method.
    HTMLFormElement.prototype.submit.call(form)
  } finally {
    form.remove()
  }
}
