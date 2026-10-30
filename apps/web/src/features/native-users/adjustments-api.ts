/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

import { api } from '@/lib/api'

import {
  isCurrentNativeUsersScope,
  nativeUsersOptions,
  type NativeUsersScope,
} from './access'
import { NativeProfileFailure } from './profile-api'

const id = z
  .string()
  .regex(/^[1-9][0-9]{0,18}$/)
  .refine(
    (value) =>
      /^[1-9][0-9]{0,18}$/.test(value) && BigInt(value) <= 9223372036854775807n
  )
const delta = z
  .string()
  .regex(/^-?[1-9][0-9]{0,18}$/)
  .refine((value) => {
    if (!/^-?[1-9][0-9]{0,18}$/.test(value)) return false
    const units = BigInt(value)
    return units >= -9223372036854775807n && units <= 9223372036854775807n
  })
export const adjustmentDraftSchema = z
  .object({
    operation_key: z.string().regex(/^[a-zA-Z0-9_-]{32,64}$/),
    delta_units: delta,
    reason: z.string().refine(
      (value) =>
        value.trim().length > 0 &&
        [...value].length <= 500 &&
        new TextEncoder().encode(value).length <= 1000 &&
        !Array.from(value).some((character) => {
          const code = character.charCodeAt(0)
          return code <= 31 || (code >= 127 && code <= 159)
        })
    ),
  })
  .strict()
const fields = {
  contract: z.literal('core.admin-user-adjustment.v1'),
  operation_key: adjustmentDraftSchema.shape.operation_key,
  user_id: id,
  delta_units: delta,
}
const receiptSchema = z.discriminatedUnion('status', [
  z
    .object({
      ...fields,
      status: z.literal('posted'),
      journal_id: id,
      rejection: z.null(),
    })
    .strict(),
  z
    .object({
      ...fields,
      status: z.literal('rejected'),
      journal_id: z.null(),
      rejection: z.enum([
        'not_found',
        'insufficient_funds',
        'invalid_original',
        'exceeds_remaining',
        'already_finalized',
        'already_compensated',
        'amount_overflow',
        'duplicate_payment',
        'duplicate_refund',
      ]),
    })
    .strict(),
])
export type AdjustmentDraft = z.infer<typeof adjustmentDraftSchema>
export type AdjustmentReceipt = z.infer<typeof receiptSchema>
function current(scope: NativeUsersScope) {
  if (!isCurrentNativeUsersScope(scope)) {
    throw new NativeProfileFailure('session_changed')
  }
}
function path(userId: string) {
  if (!id.safeParse(userId).success) throw new NativeProfileFailure('invalid')
  return `/core/v1/admin/user-balances/${userId}/adjustments`
}
function failure(error: unknown, write: boolean) {
  if (error instanceof NativeProfileFailure) return error
  const response =
    typeof error === 'object' && error !== null && 'response' in error
      ? error.response
      : undefined
  const status =
    typeof response === 'object' && response !== null && 'status' in response
      ? response.status
      : undefined
  return new NativeProfileFailure(
    status === 403 || status === 401
      ? 'forbidden'
      : status === 404
        ? 'not_found'
        : status === 409
          ? 'conflict'
          : status === 400
            ? 'invalid'
            : write
              ? 'unknown'
              : 'unavailable'
  )
}
function receipt(data: unknown, userId: string, draft: AdjustmentDraft) {
  const parsed = receiptSchema.safeParse(data)
  if (
    !parsed.success ||
    parsed.data.user_id !== userId ||
    parsed.data.operation_key !== draft.operation_key ||
    parsed.data.delta_units !== draft.delta_units
  ) {
    throw new NativeProfileFailure('unknown')
  }
  return parsed.data
}
export async function adjustUserBalance(
  scope: NativeUsersScope,
  userId: string,
  draft: AdjustmentDraft
) {
  current(scope)
  if (!adjustmentDraftSchema.safeParse(draft).success) {
    throw new NativeProfileFailure('invalid')
  }
  try {
    const result = await api.post<unknown>(
      path(userId),
      draft,
      nativeUsersOptions(scope)
    )
    current(scope)
    return receipt(result.data, userId, draft)
  } catch (error) {
    current(scope)
    throw failure(error, true)
  }
}
export async function readAdjustmentReceipt(
  scope: NativeUsersScope,
  userId: string,
  draft: AdjustmentDraft
) {
  current(scope)
  if (!adjustmentDraftSchema.safeParse(draft).success) {
    throw new NativeProfileFailure('invalid')
  }
  try {
    const result = await api.get<unknown>(
      `${path(userId)}/${draft.operation_key}`,
      nativeUsersOptions(scope)
    )
    current(scope)
    return receipt(result.data, userId, draft)
  } catch (error) {
    current(scope)
    throw failure(error, false)
  }
}
