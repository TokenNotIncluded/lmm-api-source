/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

export const checkinCapabilitiesResponseSchema = z.object({
  success: z.literal(true),
  data: z.object({ enabled: z.boolean() }),
})

const units = z.string().regex(/^(0|[1-9]\d*)$/)
const integer = z.number().int().nonnegative().safe()
export const checkinMonthSchema = z
  .string()
  .regex(/^(?!0000)\d{4}-(0[1-9]|1[0-2])$/)

const recordSchema = z.object({
  checkin_date: z.iso.date(),
  quota_awarded: units,
  created_at: integer,
  journal_id: integer.positive().nullable(),
  origin: z.enum(['native', 'history']),
  policy_revision: integer.positive().nullable(),
})

const rewardRangeSchema = z
  .object({
    reward_multiplier: z.number().nonnegative(),
    base_min_quota: units,
    base_max_quota: units,
    min_quota: units,
    max_quota: units,
  })
  .refine((range) => BigInt(range.min_quota) <= BigInt(range.max_quota))

const nativeStatusSchema = z
  .object({
    enabled: z.boolean(),
    today: z.iso.date(),
    month: checkinMonthSchema,
    calendar_offset_seconds: z.number().int().min(-86400).max(86400),
    policy_revision: integer.positive().nullable(),
    checked_in_today: z.boolean(),
    pending: z.boolean().default(false),
    reward_range: rewardRangeSchema.nullable(),
    today_record: recordSchema.nullable(),
    records: z.array(recordSchema).max(31),
    total_checkins: integer,
    total_quota_awarded: units,
  })
  .refine(
    (data) =>
      data.checked_in_today === (data.today_record !== null) &&
      (data.today_record === null ||
        data.today_record.checkin_date === data.today) &&
      data.records.every((r) => r.checkin_date.startsWith(data.month)) &&
      new Set(data.records.map((r) => r.checkin_date)).size ===
        data.records.length
  )
  .transform((data) => ({
    kind: 'native' as const,
    enabled: data.enabled,
    today: data.today,
    month: data.month,
    calendar_offset_seconds: data.calendar_offset_seconds,
    reward_range: data.reward_range,
    today_record: data.today_record,
    pending: data.pending,
    stats: {
      checked_in_today: data.checked_in_today,
      total_checkins: data.total_checkins,
      total_quota: data.total_quota_awarded,
      checkin_count: data.records.length,
      records: data.records,
    },
  }))

export const checkinFailureSchema = z.object({
  success: z.literal(false),
  message: z.string().optional(),
  error: z.object({ code: z.string() }).optional(),
})
export const checkinStatusResponseSchema = z.discriminatedUnion('success', [
  checkinFailureSchema,
  z.object({
    success: z.literal(true),
    data: nativeStatusSchema,
  }),
])
export const checkinClaimResponseSchema = z.discriminatedUnion('success', [
  checkinFailureSchema,
  z.object({
    success: z.literal(true),
    data: z
      .object({ already_claimed: z.boolean(), record: recordSchema })
      .transform((data) => ({ kind: 'native' as const, ...data })),
  }),
])

export const checkinChallengeResponseSchema = z.discriminatedUnion('success', [
  checkinFailureSchema,
  z.object({
    success: z.literal(true),
    data: z
      .object({
        challenge_id: z.string().min(1),
        operation_key: z.string().min(16).max(80),
        day: z.iso.date(),
        expires_at: integer.positive(),
        turnstile_site_key: z.string().min(1),
        turnstile_action: z.literal('daily_checkin'),
        turnstile_cdata: z.string().min(1),
        passkey_required: z.boolean(),
        passkey_options: z.record(z.string(), z.unknown()).nullable(),
      })
      .refine(
        (challenge) =>
          !challenge.passkey_required || challenge.passkey_options !== null
      ),
  }),
])

export type CheckinChallenge = Extract<
  z.infer<typeof checkinChallengeResponseSchema>,
  { success: true }
>['data']

export interface CheckinClaimInput {
  operation_key: string
  challenge_id?: string
  turnstile_token?: string
  passkey_assertion?: Record<string, unknown>
}

export type CheckinStatusResponse = Extract<
  z.infer<typeof checkinStatusResponseSchema>,
  { success: true }
>['data']
export type CheckinResponse = Extract<
  z.infer<typeof checkinClaimResponseSchema>,
  { success: true }
>['data']
