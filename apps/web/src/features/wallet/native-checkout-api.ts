/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import axios from 'axios'
import { z } from 'zod'

import { api, type ApiRequestConfig } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

import {
  nativeChannelSchema,
  nativeCheckoutSchema,
  nativeQuoteSchema,
  validateNativeCheckout,
  validateNativeQuote,
  type PendingNativeCheckout,
  type NativeQuote,
} from './lib/native-checkout'

const envelope = z.object({ success: z.literal(true), data: z.unknown() })
const channels = z
  .object({
    available: z.boolean(),
    channels: z.array(nativeChannelSchema),
    pancake_portal_available: z.boolean().default(false),
  })
  .refine((catalog) => catalog.available === catalog.channels.length > 0)
const checkoutDisabled = z.object({
  success: z.literal(false),
  code: z.literal('PAYMENT_DISABLED'),
})
function config(write = false): ApiRequestConfig {
  const { user, session } = useAuthStore.getState().auth
  return {
    authScope: { userId: user?.id, sessionId: session?.sid },
    skipAuthRefresh: write,
    skipBusinessError: true,
    skipErrorHandler: true,
    timeout: 15_000,
  }
}
export async function getNativePaymentChannels(signal?: AbortSignal) {
  try {
    const result = await api.get('/api/user/payments/channels', {
      ...config(),
      signal,
    })
    return {
      ...channels.parse(envelope.parse(result.data).data),
      checkoutConfigured: true,
    }
  } catch (error) {
    if (
      axios.isAxiosError(error) &&
      error.response?.status === 503 &&
      checkoutDisabled.safeParse(error.response.data).success
    ) {
      return {
        available: false,
        channels: [],
        pancake_portal_available: false,
        checkoutConfigured: false,
      }
    }
    throw error
  }
}
export async function quoteNativePayment(pending: PendingNativeCheckout) {
  const result = await api.post(
    '/api/user/payments/quote',
    pending.request,
    config(true)
  )
  return validateNativeQuote(
    nativeQuoteSchema.parse(envelope.parse(result.data).data),
    pending
  )
}
export async function checkoutNativePayment(
  pending: PendingNativeCheckout & { quote: NativeQuote }
) {
  const result = await api.post(
    '/api/user/payments/checkout',
    {
      ...pending.request,
      order_id: pending.quote.order_id,
      expected_credit_units: pending.quote.credit_units,
      expected_amount_minor: pending.quote.amount_minor,
    },
    config(true)
  )
  return validateNativeCheckout(
    nativeCheckoutSchema.parse(envelope.parse(result.data).data),
    pending.quote
  )
}
export async function getNativePaymentStatus(
  quote: NativeQuote,
  signal?: AbortSignal
) {
  const result = await api.get(
    `/api/user/payments/checkout/${encodeURIComponent(quote.order_id)}`,
    { ...config(), signal }
  )
  return validateNativeCheckout(
    nativeCheckoutSchema.parse(envelope.parse(result.data).data),
    quote
  )
}
