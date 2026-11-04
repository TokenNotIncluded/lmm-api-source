/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

const unsigned = z.string().regex(/^(0|[1-9]\d*)$/)
const signed = z.string().regex(/^(0|-?[1-9]\d*)$/)
const timestamp = z.number().int().nonnegative().max(8_640_000_000_000)
const order = z
  .object({
    id: z.string().min(1),
    service: z.string(),
    country_id: z.number().int().nonnegative().max(Number.MAX_SAFE_INTEGER),
    operator: z.string(),
    status: z.enum(['completed', 'cancelled', 'failed']),
    quoted_price_usd: z
      .string()
      .regex(/^\d+(\.\d+)?$/)
      .nullable(),
    raw_reserved_units: unsigned,
    raw_refunded_units: unsigned,
    raw_net_debit_units: unsigned,
    created_at: timestamp,
    completed_at: timestamp.nullable(),
    ledger: z.array(
      z.object({
        entry_type: z.enum(['reserve', 'refund']),
        raw_amount_units: signed,
        created_at: timestamp,
      })
    ),
  })
  .refine(
    (o) =>
      BigInt(o.raw_reserved_units) - BigInt(o.raw_refunded_units) ===
      BigInt(o.raw_net_debit_units)
  )
const responseSchema = z.object({
  success: z.literal(true),
  data: z.object({
    contract: z.literal('core.hero-sms-history.v1'),
    observed_at: timestamp,
    page: z.number().int().min(1).max(100_000),
    page_size: z.number().int().min(1).max(100),
    total: z.number().int().nonnegative().max(Number.MAX_SAFE_INTEGER),
    items: z.array(order).max(100),
    quota_unit: z.literal('legacy_raw_no_conversion'),
    payload_available: z.literal(false),
    supplier_operations_available: z.literal(false),
  }),
})
export type SmsHistoryScope = { userId: number; sessionId: string }
export async function readSmsHistory(
  scope: SmsHistoryScope,
  page: number,
  signal?: AbortSignal
) {
  z.number().int().min(1).max(100_000).parse(page)
  const assertCurrent = () => {
    const { user, session } = useAuthStore.getState().auth
    if (
      signal?.aborted ||
      user?.id !== scope.userId ||
      session?.sid !== scope.sessionId
    ) {
      throw new Error('SMS history session changed')
    }
  }
  assertCurrent()
  const response = await api.get<unknown>('/core/v1/hero-sms/history', {
    authScope: scope,
    params: { p: page, page_size: 20 },
    signal,
    disableDuplicate: true,
    skipAuthRefresh: true,
    skipBusinessError: true,
    skipErrorHandler: true,
  })
  assertCurrent()
  const { data } = responseSchema.parse(response.data)
  const expected = Math.min(20, Math.max(0, data.total - (page - 1) * 20))
  if (
    data.page !== page ||
    data.page_size !== 20 ||
    data.items.length !== expected ||
    new Set(data.items.map((o) => o.id)).size !== data.items.length
  ) {
    throw new Error('Invalid SMS history page')
  }
  return data
}
