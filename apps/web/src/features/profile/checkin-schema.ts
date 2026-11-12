/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

export const checkinCapabilitiesResponseSchema = z.object({
  success: z.literal(true),
  data: z.object({ enabled: z.boolean() }),
})

const units = z.string().regex(/^(0|[1-9]\d*)$/)
const legacyUnits = z.number().int().nonnegative().safe().transform(String)
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
  trust_level: integer.max(255).nullable(),
})

const rewardRangeSchema = z
  .object({
    trust_level: integer.max(255),
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
    stats: {
      checked_in_today: data.checked_in_today,
      total_checkins: data.total_checkins,
      total_quota: data.total_quota_awarded,
      checkin_count: data.records.length,
      records: data.records,
    },
  }))

const legacyRecordSchema = z.object({
  checkin_date: z.iso.date(),
  quota_awarded: legacyUnits,
})
const legacyStatusSchema = z
  .object({
    enabled: z.boolean(),
    min_quota: legacyUnits.optional(),
    max_quota: legacyUnits.optional(),
    trust_level: integer.optional(),
    stats: z.object({
      checked_in_today: z.boolean(),
      total_checkins: integer,
      total_quota: legacyUnits,
      checkin_count: integer,
      records: z.array(legacyRecordSchema).max(31),
    }),
  })
  .transform((data) => ({
    kind: 'legacy' as const,
    enabled: data.enabled,
    reward_range:
      data.min_quota !== undefined &&
      data.max_quota !== undefined &&
      data.trust_level !== undefined
        ? {
            min_quota: data.min_quota,
            max_quota: data.max_quota,
            trust_level: data.trust_level,
          }
        : null,
    stats: data.stats,
  }))

const failureSchema = z.object({
  success: z.literal(false),
  message: z.string().optional(),
})
export const checkinStatusResponseSchema = z.discriminatedUnion('success', [
  failureSchema,
  z.object({
    success: z.literal(true),
    data: z.union([nativeStatusSchema, legacyStatusSchema]),
  }),
])
export const checkinClaimResponseSchema = z.discriminatedUnion('success', [
  failureSchema,
  z.object({
    success: z.literal(true),
    data: z.union([
      z
        .object({ already_claimed: z.boolean(), record: recordSchema })
        .transform((data) => ({ kind: 'native' as const, ...data })),
      z
        .object({
          quota_awarded: legacyUnits,
          checkin_date: z.iso.date().optional(),
        })
        .transform((data) => ({ kind: 'legacy' as const, ...data })),
    ]),
  }),
])

export type CheckinStatusResponse = Extract<
  z.infer<typeof checkinStatusResponseSchema>,
  { success: true }
>['data']
export type CheckinResponse = Extract<
  z.infer<typeof checkinClaimResponseSchema>,
  { success: true }
>['data']
