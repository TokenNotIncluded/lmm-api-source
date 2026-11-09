/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

import {
  CommerceFailure,
  commerceRequest,
  exactUnits,
  nativeId,
  operationKey,
  type CommerceScope,
} from '@/features/commerce/native-client'

import { planSchema } from './native-api'

export const grantStatusSchema = z.enum([
  'active',
  'scheduled',
  'expired',
  'invalidated',
  'refunded',
])
export const grantRecordSchema = z
  .object({
    id: nativeId,
    user_id: nativeId,
    username: z.string(),
    email: z.string(),
    grant_reference: z.string().min(1),
    source: z.enum(['wallet', 'imported', 'other']),
    plan: planSchema.nullable(),
    starts_at: z.number().int().nonnegative(),
    ends_at: z.number().int().positive(),
    window_start: z.number().int().nonnegative(),
    window_end: z.number().int().positive(),
    limit_units: exactUnits,
    unlimited: z.boolean(),
    used_units: exactUnits,
    reserved_units: exactUnits,
    active: z.boolean(),
    status: grantStatusSchema,
    purchase_id: nativeId.nullable(),
    refund_journal_id: nativeId.nullable(),
    renewal_canceled: z.boolean().nullable(),
  })
  .strict()
export type NativeGrantRecord = z.infer<typeof grantRecordSchema>
export const grantRecordsSchema = z
  .object({
    contract: z.literal('core.subscriptions.v1'),
    items: z.array(grantRecordSchema).max(100),
    total: z.number().int().nonnegative().max(Number.MAX_SAFE_INTEGER),
    page: z.number().int().min(1).max(100000),
    page_size: z.number().int().min(1).max(100),
  })
  .strict()
export type NativeGrantRecords = z.infer<typeof grantRecordsSchema>
export const grantInvalidationSchema = z
  .object({
    operation_key: operationKey,
    user_id: nativeId,
    reason: z.string().trim().min(1).max(500),
  })
  .strict()
export const pendingGrantInvalidationSchema = z
  .object({
    grant_id: nativeId,
    request: grantInvalidationSchema,
  })
  .strict()
export type PendingGrantInvalidation = z.infer<
  typeof pendingGrantInvalidationSchema
>
const receiptSchema = z
  .object({
    contract: z.literal('core.subscriptions.v1'),
    operation_key: operationKey,
    subscription_id: nativeId,
    user_id: nativeId,
    invalidated_at: z.number().int().positive(),
    renewal_stopped: z.boolean(),
  })
  .strict()

export async function listNativeGrantRecords(
  scope: CommerceScope,
  input: { page: number; query: string; status: string; userId?: number }
) {
  const params = new URLSearchParams({
    page: String(input.page),
    page_size: '20',
    query: input.query,
    status: input.status,
  })
  if (input.userId !== undefined) {
    params.set('user_id', String(nativeId.parse(input.userId)))
  }
  const result = await commerceRequest(
    scope,
    'get',
    `/core/v1/subscriptions/admin/records?${params}`,
    grantRecordsSchema
  )
  if (
    result.page !== input.page ||
    result.page_size !== 20 ||
    (input.userId !== undefined &&
      result.items.some((item) => item.user_id !== input.userId))
  ) {
    throw new CommerceFailure('INVALID_RESPONSE')
  }
  return result
}

export async function invalidateNativeGrant(
  scope: CommerceScope,
  command: PendingGrantInvalidation
) {
  pendingGrantInvalidationSchema.parse(command)
  const result = await commerceRequest(
    scope,
    'post',
    `/core/v1/subscriptions/admin/grants/${command.grant_id}/invalidate`,
    receiptSchema,
    command.request
  )
  if (
    result.subscription_id !== command.grant_id ||
    result.user_id !== command.request.user_id ||
    result.operation_key !== command.request.operation_key
  ) {
    throw new CommerceFailure('INVALID_RESPONSE')
  }
  return result
}
