/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

const monthSchema = z.string().regex(/^(?!0000)\d{4}-(0[1-9]|1[0-2])$/)
export const checkinHistorySchema = z.object({
  success: z.literal(true),
  data: z.object({
    contract: z.literal('core.checkin-history.v1'),
    month: monthSchema,
    observed_at: z.number().int().nonnegative().max(Number.MAX_SAFE_INTEGER),
    total_checkins: z.number().int().nonnegative().max(Number.MAX_SAFE_INTEGER),
    total_raw_quota_awarded: z.string().regex(/^(0|[1-9]\d*)$/),
    award_unit: z.literal('legacy_raw_no_conversion'),
    native_claim_available: z.literal(false),
    calendar_timezone: z.null(),
    records: z
      .array(
        z.object({
          checkin_date: z.iso.date(),
          raw_quota_awarded: z
            .number()
            .int()
            .nonnegative()
            .max(Number.MAX_SAFE_INTEGER),
          source_created_at: z
            .number()
            .int()
            .nonnegative()
            .max(Number.MAX_SAFE_INTEGER),
        })
      )
      .max(31),
  }),
})
export type CheckinHistoryScope = { userId: number; sessionId: string }

export async function readCheckinHistory(
  scope: CheckinHistoryScope,
  month: string,
  signal?: AbortSignal
) {
  monthSchema.parse(month)
  const assertCurrent = () => {
    const { user, session } = useAuthStore.getState().auth
    if (
      signal?.aborted ||
      user?.id !== scope.userId ||
      session?.sid !== scope.sessionId
    ) {
      throw new Error('Check-in history session changed')
    }
  }
  assertCurrent()
  const response = await api.get<unknown>('/core/v1/checkins/history', {
    authScope: scope,
    params: { month },
    signal,
    disableDuplicate: true,
    skipAuthRefresh: true,
    skipBusinessError: true,
    skipErrorHandler: true,
  })
  assertCurrent()
  const { data } = checkinHistorySchema.parse(response.data)
  if (
    data.month !== month ||
    data.records.some((r) => !r.checkin_date.startsWith(month)) ||
    new Set(data.records.map((r) => r.checkin_date)).size !==
      data.records.length
  ) {
    throw new Error('Invalid check-in history period')
  }
  return data
}
