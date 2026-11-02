/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

import {
  nativeRequest,
  type NativeRoutingScope,
  type NativeVersion,
} from './native-api'
import { balanceObservationSchema } from './types'

export type NativeChannelSort =
  | 'id'
  | 'name'
  | 'priority'
  | 'lifetime'
  | 'balance'
  | 'response_time'
  | 'test_time'
export type NativeChannelSortInput = {
  sort: NativeChannelSort
  order: 'asc' | 'desc'
  unit: string
  page: number
  search: string
  snapshotDigest?: string
}
const integer = z.string().regex(/^-?(0|[1-9]\d*)$/)
const time = z.number().int().safe().nonnegative()
export const channelStatisticsSchema = z
  .object({
    id: z.number().int().positive().safe(),
    lifetime: z
      .object({
        complete: z.boolean(),
        total_usd: z
          .string()
          .regex(/^-?\d+(?:\.\d+)?$/)
          .nullable(),
        exact_usd_numerator: integer.nullable(),
        exact_usd_denominator: z
          .string()
          .regex(/^[1-9]\d*$/)
          .nullable(),
      })
      .refine((v) =>
        v.complete
          ? v.total_usd !== null &&
            v.exact_usd_numerator !== null &&
            v.exact_usd_denominator !== null
          : v.total_usd === null &&
            v.exact_usd_numerator === null &&
            v.exact_usd_denominator === null
      ),
    observation: balanceObservationSchema.nullable(),
    response_time: time.nullable(),
    test_time: time.nullable(),
    catalog_observation: z
      .object({
        scope: z.literal('model_catalog'),
        inference_tested: z.literal(false),
        result: z.enum(['success', 'failed']),
        duration_ms: time,
        observed_at: time,
      })
      .nullable(),
  })
  .refine((v) =>
    v.catalog_observation
      ? v.test_time === v.catalog_observation.observed_at &&
        v.response_time ===
          (v.catalog_observation.result === 'success'
            ? v.catalog_observation.duration_ms
            : null)
      : v.test_time === null && v.response_time === null
  )
export type NativeChannelStatistics = z.infer<typeof channelStatisticsSchema>
const pageSchema = z.object({
  items: z.array(channelStatisticsSchema),
  total: z.number().int().safe().nonnegative(),
  page: z.number().int().positive().safe(),
  page_size: z.literal(25),
  config_version: z.number().int().positive().safe(),
  pricing_version: z.number().int().positive().safe(),
  snapshot_digest: z.string().regex(/^[a-f0-9]{64}$/),
  pagination_consistency: z.enum(['single_request', 'digest_checked']),
  balance_units: z.array(z.string()),
  sort_unit: z.string().nullable(),
  statistics_snapshot_at: time,
  statistics_scope: z.literal('channel_display_v1'),
  probe_scope: z.literal('model_catalog'),
  inference_tested: z.literal(false),
})
export async function getNativeChannelPage(
  scope: NativeRoutingScope,
  expected: NativeVersion,
  input: NativeChannelSortInput,
  signal?: AbortSignal
) {
  z.number().int().positive().safe().parse(input.page)
  z.enum([
    'id',
    'name',
    'priority',
    'lifetime',
    'balance',
    'response_time',
    'test_time',
  ]).parse(input.sort)
  z.enum(['asc', 'desc']).parse(input.order)
  if (input.sort === 'balance') {
    z.string()
      .min(1)
      .refine(
        (s) => s.trim() === s && [...s].length <= 32 && !/\p{Cc}/u.test(s)
      )
      .parse(input.unit)
  }
  const params = new URLSearchParams({
    statistics: 'true',
    p: String(input.page),
    page_size: '25',
    keyword: input.search,
    sort_by: input.sort,
    sort_order: input.order,
  })
  if (input.sort === 'balance') params.set('sort_unit', input.unit)
  if (input.snapshotDigest !== undefined) {
    params.set(
      'snapshot_digest',
      z
        .string()
        .regex(/^[a-f0-9]{64}$/)
        .parse(input.snapshotDigest)
    )
  }
  const result = pageSchema.parse(
    await nativeRequest<unknown>(
      scope,
      'GET',
      `/api/channel?${params}`,
      undefined,
      { signal }
    )
  )
  if (
    result.config_version !== expected.routing ||
    result.pricing_version !== expected.pricing ||
    result.page !== input.page ||
    (input.sort === 'balance' && result.sort_unit !== input.unit) ||
    (input.snapshotDigest
      ? result.snapshot_digest !== input.snapshotDigest ||
        result.pagination_consistency !== 'digest_checked'
      : result.pagination_consistency !== 'single_request')
  ) {
    throw new Error('The routing snapshot changed. Reload before continuing.')
  }
  return result
}

export function isChannelSnapshotChanged(error: unknown): boolean {
  return z
    .object({
      response: z.object({
        status: z.literal(409),
        data: z.object({ code: z.literal('CHANNEL_SNAPSHOT_CHANGED') }),
      }),
    })
    .safeParse(error).success
}
