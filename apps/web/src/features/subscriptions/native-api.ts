/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

import {
  CommerceFailure,
  commerceRequest,
  exactUnits,
  nativeId,
  nativeToken,
  operationKey,
  positiveUnits,
  type CommerceScope,
} from '@/features/commerce/native-client'

export const planTermsSchema = z
  .object({
    title: z.string().trim().min(1).max(160),
    price_units: positiveUnits,
    duration_seconds: z.number().int().min(60).max(315576000),
    reset_period: z.enum([
      'grant',
      'day',
      'week',
      'month',
      'anniversary_month',
      'custom',
    ]),
    reset_seconds: z.number().int().nonnegative(),
    limit_units: positiveUnits,
    models: z.array(z.string().min(1).max(128)).min(1).max(256),
    groups: z.array(z.string().min(1).max(128)).min(1).max(256),
    max_purchases_per_user: z.number().int().min(0).max(10000),
    enabled: z.boolean(),
  })
  .strict()
export const planSchema = z
  .object({
    id: nativeId,
    revision: nativeId,
    archived: z.boolean(),
    terms: planTermsSchema,
  })
  .strict()
export type NativePlan = z.infer<typeof planSchema>
export type NativePlanTerms = z.infer<typeof planTermsSchema>
export const savePlanSchema = z
  .object({
    operation_key: operationKey,
    expected_revision: nativeId.nullable(),
    terms: planTermsSchema,
  })
  .strict()
export type NativeSavePlan = z.infer<typeof savePlanSchema>
export const quoteRequestSchema = z
  .object({
    operation_key: operationKey,
    plan_id: nativeId,
    renew_from: nativeId.nullable(),
  })
  .strict()
export const quoteSchema = z
  .object({
    id: nativeToken,
    plan: planSchema,
    starts_at: z.number().int(),
    ends_at: z.number().int(),
    expires_at: z.number().int(),
    renew_from: nativeId.nullable(),
  })
  .strict()
export type NativeQuote = z.infer<typeof quoteSchema>
export const receiptSchema = z
  .object({
    quote_id: nativeToken,
    status: z.enum(['posted', 'rejected', 'refunded']),
    purchase_id: nativeId.nullable(),
    subscription_id: nativeId.nullable(),
    journal_id: nativeId.nullable(),
    amount_units: positiveUnits,
    rejection: z.string().nullable(),
  })
  .strict()
  .refine((value) =>
    value.status === 'rejected'
      ? value.purchase_id === null &&
        value.subscription_id === null &&
        value.journal_id === null &&
        value.rejection !== null
      : value.purchase_id !== null &&
        value.subscription_id !== null &&
        value.journal_id !== null &&
        value.rejection === null
  )
export type NativeReceipt = z.infer<typeof receiptSchema>
export const purchaseSchema = z
  .object({
    id: nativeId,
    subscription_id: nativeId,
    plan: planSchema,
    starts_at: z.number().int(),
    ends_at: z.number().int(),
    used_units: exactUnits,
    reserved_units: exactUnits,
    entitlement_active: z.boolean(),
    renewal_canceled: z.boolean(),
    canceled_at: z.number().int().nullable(),
    refund_journal_id: nativeId.nullable(),
  })
  .strict()
export type NativePurchase = z.infer<typeof purchaseSchema>
export const refundQuoteSchema = z
  .object({
    id: nativeToken,
    purchase_id: nativeId,
    amount_units: positiveUnits,
    expires_at: z.number().int(),
  })
  .strict()
export const purchasePendingSchema = z.discriminatedUnion('kind', [
  z
    .object({
      kind: z.literal('purchase'),
      request: quoteRequestSchema,
      quote: quoteSchema.optional(),
      confirmed: z.boolean(),
    })
    .strict(),
  z
    .object({
      kind: z.literal('refund'),
      purchase_id: nativeId,
      quote: refundQuoteSchema,
      confirmed: z.boolean(),
    })
    .strict(),
])
export type SubscriptionPending = z.infer<typeof purchasePendingSchema>
const base = '/core/v1/subscriptions'
export async function listNativePlans(scope: CommerceScope, admin = false) {
  const response = await commerceRequest(
    scope,
    'get',
    `${base}/${admin ? 'admin/' : ''}plans`,
    z.object({
      contract: z.literal('core.subscriptions.v1'),
      plans: z.array(planSchema),
      payment_methods: z.array(z.string()).optional(),
      automatic_renewal: z.boolean().optional(),
    })
  )
  return response.plans
}
export async function listNativePurchases(scope: CommerceScope) {
  return (
    await commerceRequest(
      scope,
      'get',
      `${base}/purchases`,
      z.object({
        contract: z.literal('core.subscriptions.v1'),
        purchases: z.array(purchaseSchema),
      })
    )
  ).purchases
}
export async function quoteNativeSubscription(
  scope: CommerceScope,
  request: z.infer<typeof quoteRequestSchema>
) {
  quoteRequestSchema.parse(request)
  const result = await commerceRequest(
    scope,
    'post',
    `${base}/quotes`,
    quoteSchema,
    request
  )
  if (
    result.plan.id !== request.plan_id ||
    result.renew_from !== request.renew_from ||
    result.ends_at <= result.starts_at
  ) {
    throw new CommerceFailure('INVALID_RESPONSE')
  }
  return result
}
export async function confirmNativeSubscription(
  scope: CommerceScope,
  quote: NativeQuote
) {
  const result = await commerceRequest(
    scope,
    'post',
    `${base}/quotes/${quote.id}/confirm`,
    receiptSchema
  )
  if (
    result.quote_id !== quote.id ||
    result.amount_units !== quote.plan.terms.price_units ||
    result.status === 'refunded'
  ) {
    throw new CommerceFailure('INVALID_RESPONSE')
  }
  return result
}
export async function previewNativeRefund(
  scope: CommerceScope,
  purchase: NativePurchase
) {
  const result = await commerceRequest(
    scope,
    'post',
    `${base}/purchases/${purchase.id}/refund-preview`,
    refundQuoteSchema
  )
  if (
    result.purchase_id !== purchase.id ||
    result.amount_units !== purchase.plan.terms.price_units
  ) {
    throw new CommerceFailure('INVALID_RESPONSE')
  }
  return result
}
export async function confirmNativeRefund(
  scope: CommerceScope,
  quote: z.infer<typeof refundQuoteSchema>
) {
  const result = await commerceRequest(
    scope,
    'post',
    `${base}/refund-quotes/${quote.id}/confirm`,
    receiptSchema
  )
  if (
    result.quote_id !== quote.id ||
    result.purchase_id !== quote.purchase_id ||
    result.amount_units !== quote.amount_units ||
    result.status !== 'refunded'
  ) {
    throw new CommerceFailure('INVALID_RESPONSE')
  }
  return result
}
export async function cancelNativeRenewal(
  scope: CommerceScope,
  purchase: NativePurchase
) {
  const result = await commerceRequest(
    scope,
    'post',
    `${base}/purchases/${purchase.id}/cancel-renewal`,
    purchaseSchema
  )
  if (result.id !== purchase.id || !result.renewal_canceled) {
    throw new CommerceFailure('INVALID_RESPONSE')
  }
  return result
}
export function saveNativePlan(
  scope: CommerceScope,
  id: number | null,
  request: NativeSavePlan
) {
  savePlanSchema.parse(request)
  return commerceRequest(
    scope,
    id ? 'put' : 'post',
    `${base}/admin/plans${id ? `/${id}` : ''}`,
    planSchema,
    request
  )
}
export function archiveNativePlan(
  scope: CommerceScope,
  plan: NativePlan,
  operation_key: string
) {
  return commerceRequest(
    scope,
    'post',
    `${base}/admin/plans/${plan.id}/archive`,
    planSchema,
    { operation_key, expected_revision: plan.revision }
  )
}
