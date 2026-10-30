/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
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
const channels = z.object({
  available: z.boolean(),
  channels: z.array(nativeChannelSchema),
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
  const result = await api.get('/api/user/payments/channels', {
    ...config(),
    signal,
  })
  return channels.parse(envelope.parse(result.data).data)
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
    },
    config(true)
  )
  return validateNativeCheckout(
    nativeCheckoutSchema.parse(envelope.parse(result.data).data),
    pending.quote
  )
}
export async function getNativePaymentStatus(quote: NativeQuote) {
  const result = await api.get(
    `/api/user/payments/checkout/${encodeURIComponent(quote.order_id)}`,
    config()
  )
  return validateNativeCheckout(
    nativeCheckoutSchema.parse(envelope.parse(result.data).data),
    quote
  )
}
