/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'
export const relayOrderStates = z.enum([
  'preview',
  'payment_pending',
  'grant_pending',
  'settlement_pending',
  'completed',
  'declined',
  'expired',
  'refund_required',
  'refund_pending',
  'needs_review',
  'refunded',
])
const positiveInteger = z.number().int().positive().max(Number.MAX_SAFE_INTEGER)
const creditsPerDollar = 500_000

const orderResponse = z.object({
  success: z.literal(true),
  data: z.object({
    order_id: z.string().regex(/^[a-f0-9]{64}$/),
    user_id: positiveInteger,
    gib: positiveInteger.max(17_179_869_183),
    unit_price_quota_per_gib: positiveInteger.optional(),
    total_price_quota: positiveInteger.optional(),
    amount_usd_cents: z
      .number()
      .int()
      .nonnegative()
      .max(Number.MAX_SAFE_INTEGER)
      .optional(),
    period: z.string().regex(/^\d{4}-(0[1-9]|1[0-2])$/),
    state: relayOrderStates,
    refund_ref: z.string().optional(),
  }),
})
export function readRelayOrder(
  payload: unknown,
  orderID: string,
  userID: number | undefined
) {
  const { data } = orderResponse.parse(payload)
  if (data.order_id !== orderID || data.user_id !== userID)
    throw new Error('Invalid relay order')

  const unit = data.unit_price_quota_per_gib
  const total = data.total_price_quota
  const cents = data.amount_usd_cents
  if (unit === undefined && total === undefined) {
    if (cents !== data.gib * 100) throw new Error('Invalid relay order')
    return {
      ...data,
      unit_price_quota_per_gib: creditsPerDollar,
      total_price_quota: data.gib * creditsPerDollar,
    }
  }
  if (
    unit === undefined ||
    total === undefined ||
    BigInt(unit) * BigInt(data.gib) !== BigInt(total) ||
    (cents !== undefined &&
      cents > 0 &&
      BigInt(cents) * 5_000n !== BigInt(total))
  )
    throw new Error('Invalid relay order')
  return { ...data, unit_price_quota_per_gib: unit, total_price_quota: total }
}

// One ledger credit is exactly two millionths of a US dollar.
export function formatRelayUSD(quota: number, locale: string): string {
  const amount = BigInt(quota)
  const denominator = BigInt(creditsPerDollar)
  const fraction = ((amount % denominator) * 2n)
    .toString()
    .padStart(6, '0')
    .replace(/0{1,4}$/, '')
  const localizedFraction = new Intl.NumberFormat(locale, {
    useGrouping: false,
    minimumIntegerDigits: fraction.length,
  }).format(BigInt(fraction))
  return new Intl.NumberFormat(locale, { style: 'currency', currency: 'USD' })
    .formatToParts(amount / denominator)
    .map((part) => (part.type === 'fraction' ? localizedFraction : part.value))
    .join('')
}

const relayPurchaseTool = '1ae22cbb-5d64-5b9d-95b2-d2ac5f934682'

export function relayOrderLink(
  toolID: string,
  execution: string,
  result: unknown
) {
  if (toolID !== relayPurchaseTool || execution !== 'succeeded')
    return undefined
  const preview = z
    .object({
      isError: z.literal(false).optional(),
      structuredContent: z.object({
        order: z.object({ order_id: z.string().regex(/^[a-f0-9]{64}$/) }),
      }),
    })
    .safeParse(result)
  if (!preview.success) return undefined
  return (
    '/tool-market?relay_order=' + preview.data.structuredContent.order.order_id
  )
}
