/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

const integer = z
  .string()
  .refine(
    (value) =>
      /^[1-9]\d{0,18}$/.test(value) && BigInt(value) <= 9223372036854775807n
  )
const currency = z.enum(['USD', 'CNY', 'EUR', 'LDC'])
export const nativeChannelSchema = z
  .object({
    id: z.string().min(1),
    provider: z.enum(['epay', 'stripe', 'creem', 'waffo', 'waffo_pancake']),
    environment: z.enum(['test', 'live']),
    currency,
    payment_type: z.string().optional(),
    product_id: z.string().optional(),
    fixed_amount_minor: integer.optional(),
  })
  .refine(
    (channel) =>
      channel.provider !== 'creem' ||
      Boolean(channel.product_id && channel.fixed_amount_minor)
  )
const requestSchema = z.object({
  channel_id: z.string().min(1),
  amount_minor: integer,
  currency,
  request_key: z.string().min(1).max(128),
})
export const nativeQuoteSchema = requestSchema.extend({
  order_id: z.string().regex(/^top_[1-9]\d*$/),
  account_id: z.string().min(1),
  provider: nativeChannelSchema.shape.provider,
  environment: nativeChannelSchema.shape.environment,
  credit_units: integer,
  credit_unit: z.literal('credit_500k_usd'),
})
export const nativeCheckoutSchema = z.object({
  order_id: nativeQuoteSchema.shape.order_id,
  state: z.enum([
    'checkout_pending',
    'awaiting_payment',
    'credit_pending',
    'credited',
    'refund_pending',
    'partially_refunded',
    'refunded',
    'review',
  ]),
  currency,
  amount_minor: integer,
  credit_units: integer,
  credit_unit: z.literal('credit_500k_usd'),
  checkout_url: z.string().optional(),
  checkout_method: z.enum(['GET', 'POST']).default('GET'),
  provider_paid: z.boolean(),
  core_credit_confirmed: z.boolean(),
})
const pendingSchema = z.object({
  channel: nativeChannelSchema,
  request: requestSchema,
  quote: nativeQuoteSchema.optional(),
  submitted: z.boolean(),
})
export type NativeChannel = z.infer<typeof nativeChannelSchema>
export type NativePaymentRequest = z.infer<typeof requestSchema>
export type NativeQuote = z.infer<typeof nativeQuoteSchema>
export type NativeCheckout = z.infer<typeof nativeCheckoutSchema>
export type PendingNativeCheckout = z.infer<typeof pendingSchema>

export function nativeAmountMinor(value: string): string | null {
  const match = /^(\d+)(?:\.(\d{1,2}))?$/.exec(value.trim())
  if (!match) return null
  const minor = `${match[1]}${(match[2] ?? '').padEnd(2, '0')}`.replace(
    /^0+/,
    ''
  )
  return integer.safeParse(minor).success ? minor : null
}

export function nativeAmountDisplay(minor: string): string {
  const digits = minor.padStart(3, '0')
  return `${digits.slice(0, -2)}.${digits.slice(-2)}`
}

export function validateNativeQuote(
  quote: NativeQuote,
  pending: PendingNativeCheckout
) {
  const { request, channel } = pending
  if (
    quote.channel_id !== request.channel_id ||
    quote.request_key !== request.request_key ||
    quote.currency !== request.currency ||
    quote.amount_minor !== request.amount_minor ||
    quote.provider !== channel.provider ||
    quote.environment !== channel.environment
  ) {
    throw new Error('Payment quote does not match the saved request')
  }
  return quote
}

export function validateNativeCheckout(
  result: NativeCheckout,
  quote: NativeQuote
) {
  if (
    result.order_id !== quote.order_id ||
    result.currency !== quote.currency ||
    result.amount_minor !== quote.amount_minor ||
    result.credit_units !== quote.credit_units ||
    result.credit_unit !== quote.credit_unit
  ) {
    throw new Error('Payment status does not match the reviewed quote')
  }
  return result
}

export function nativeCheckoutURL(value: string | undefined): string | null {
  if (!value) return null
  try {
    const url = new URL(value)
    return url.protocol === 'https:' && !url.username && !url.password
      ? url.href
      : null
  } catch {
    return null
  }
}

export function nativeCheckoutFinished(result: NativeCheckout | null) {
  return (
    result?.state === 'refunded' ||
    (result?.state === 'credited' && result.core_credit_confirmed)
  )
}

const storageKey = (owner: string) => `lmm-native-payment:${owner}`

export function readNativeCheckout(
  owner: string,
  storage: Pick<Storage, 'getItem'>
) {
  const value = storage.getItem(storageKey(owner))
  if (value === null) return null
  const pending = pendingSchema.parse(JSON.parse(value))
  if (
    pending.channel.id !== pending.request.channel_id ||
    pending.channel.currency !== pending.request.currency ||
    (pending.channel.fixed_amount_minor &&
      pending.channel.fixed_amount_minor !== pending.request.amount_minor) ||
    (pending.submitted && !pending.quote)
  ) {
    throw new Error('Saved payment request is inconsistent')
  }
  if (pending.quote) validateNativeQuote(pending.quote, pending)
  return pending
}

export function saveNativeCheckout(
  owner: string,
  pending: PendingNativeCheckout | null,
  storage: Pick<Storage, 'setItem' | 'removeItem'>
) {
  if (pending) storage.setItem(storageKey(owner), JSON.stringify(pending))
  else storage.removeItem(storageKey(owner))
}
