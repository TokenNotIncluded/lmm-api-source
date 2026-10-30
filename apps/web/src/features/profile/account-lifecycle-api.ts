/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

import { api } from '@/lib/api'
import { withAuthenticatedCookieWrite } from '@/lib/auth-session'
import { useAuthStore } from '@/stores/auth-store'

const version = z
  .string()
  .regex(/^[1-9][0-9]{0,18}$/)
  .pipe(z.string().refine((value) => BigInt(value) <= 9223372036854775807n))
const operationKey = z.string().regex(/^[a-f0-9]{64}$/)
export const closurePreviewSchema = z
  .object({
    contract: z.literal('core.account-closure.v1'),
    expected_auth_version: version,
    username: z.string().min(1),
    allowed: z.boolean(),
    blockers: z.array(
      z.enum([
        'privileged_account',
        'unsettled_balance',
        'financial_history_unavailable',
        'owned_team',
        'model_request_in_progress',
        'active_subscription',
        'payment_or_refund_pending',
        'order_in_progress',
        'transfer_pending',
      ])
    ),
  })
  .strict()
  .refine((value) => value.allowed === (value.blockers.length === 0))
const receiptSchema = z
  .object({
    contract: z.literal('core.account-closure.v1'),
    operation_key: operationKey,
    closed: z.literal(true),
  })
  .strict()
const pendingSchema = z
  .object({
    user_id: z.number().int().positive().max(Number.MAX_SAFE_INTEGER),
    operation_key: operationKey,
  })
  .strict()
export type AccountClosurePreview = z.infer<typeof closurePreviewSchema>
export type AccountClosurePending = z.infer<typeof pendingSchema>
export type ClosureScope = { userId: number; sessionId: string }
const PENDING = 'lmm-account-closure-pending-v1'

export class AccountClosureFailure extends Error {
  constructor(
    readonly kind:
      | 'unknown'
      | 'unavailable'
      | 'session'
      | 'conflict'
      | 'invalid',
    readonly preview?: AccountClosurePreview
  ) {
    super(kind)
  }
}
export function closureScope(): ClosureScope {
  const auth = useAuthStore.getState().auth
  if (!auth.user || !auth.session) throw new AccountClosureFailure('session')
  return { userId: auth.user.id, sessionId: auth.session.sid }
}
function current(scope: ClosureScope) {
  const auth = useAuthStore.getState().auth
  if (auth.user?.id !== scope.userId || auth.session?.sid !== scope.sessionId) {
    throw new AccountClosureFailure('session')
  }
}
export function readPendingClosure(
  userId?: number
): AccountClosurePending | null {
  const raw = window.sessionStorage.getItem(
    userId === undefined ? PENDING : `${PENDING}:${userId}`
  )
  return raw ? pendingSchema.parse(JSON.parse(raw)) : null
}
export function prepareClosure(scope: ClosureScope): AccountClosurePending {
  current(scope)
  const prior = readPendingClosure(scope.userId)
  if (prior) {
    if (prior.user_id !== scope.userId) {
      throw new AccountClosureFailure('conflict')
    }
    window.sessionStorage.setItem(PENDING, JSON.stringify(prior))
    return prior
  }
  const bytes = crypto.getRandomValues(new Uint8Array(32))
  const pending = {
    user_id: scope.userId,
    operation_key: Array.from(bytes, (byte) =>
      byte.toString(16).padStart(2, '0')
    ).join(''),
  }
  window.sessionStorage.setItem(
    `${PENDING}:${scope.userId}`,
    JSON.stringify(pending)
  )
  window.sessionStorage.setItem(PENDING, JSON.stringify(pending))
  if (
    readPendingClosure(scope.userId)?.operation_key !== pending.operation_key
  ) {
    throw new AccountClosureFailure('unavailable')
  }
  return pending
}
export function clearPendingClosure(receiptKey: string, userId: number) {
  if (readPendingClosure(userId)?.operation_key === receiptKey) {
    window.sessionStorage.removeItem(`${PENDING}:${userId}`)
  }
  if (readPendingClosure()?.operation_key === receiptKey) {
    window.sessionStorage.removeItem(PENDING)
  }
}
function failure(error: unknown, write: boolean): AccountClosureFailure {
  if (error instanceof AccountClosureFailure) return error
  const response =
    typeof error === 'object' && error !== null && 'response' in error
      ? error.response
      : undefined
  const status =
    typeof response === 'object' && response !== null && 'status' in response
      ? response.status
      : undefined
  const body =
    typeof response === 'object' && response !== null && 'data' in response
      ? response.data
      : undefined
  const data =
    typeof body === 'object' && body !== null && 'data' in body
      ? body.data
      : undefined
  const view = closurePreviewSchema.safeParse(data)
  return new AccountClosureFailure(
    status === 401 || status === 403
      ? 'session'
      : status === 400
        ? 'invalid'
        : status === 409
          ? 'conflict'
          : write
            ? 'unknown'
            : 'unavailable',
    view.success ? view.data : undefined
  )
}
export async function readAccountClosure(
  scope: ClosureScope
): Promise<AccountClosurePreview> {
  current(scope)
  try {
    const response = await api.get('/api/user/self/closure', {
      authScope: scope,
      skipAuthRefresh: true,
      skipErrorHandler: true,
      timeout: 10000,
    })
    current(scope)
    if (response.data?.success !== true) {
      throw new AccountClosureFailure('unavailable')
    }
    return closurePreviewSchema.parse(response.data.data)
  } catch (error) {
    current(scope)
    throw failure(error, false)
  }
}
export async function closeAccount(
  scope: ClosureScope,
  pending: AccountClosurePending,
  preview: AccountClosurePreview,
  confirmation: string
) {
  current(scope)
  if (
    pending.user_id !== scope.userId ||
    confirmation !== preview.username ||
    !preview.allowed
  ) {
    throw new AccountClosureFailure('invalid')
  }
  // The durable draft is required before the first network dispatch. No access
  // bearer, refresh secret or password is put into sessionStorage.
  if (
    readPendingClosure(scope.userId)?.operation_key !== pending.operation_key
  ) {
    throw new AccountClosureFailure('invalid')
  }
  try {
    return await withAuthenticatedCookieWrite(async (config) => {
      current(scope)
      const response = await api.delete('/api/user/self', {
        ...config,
        skipErrorHandler: true,
        skipBusinessError: true,
        data: {
          expected_auth_version: preview.expected_auth_version,
          confirmation,
          operation_key: pending.operation_key,
        },
      })
      current(scope)
      if (response.data?.success !== true) {
        throw new AccountClosureFailure('unknown')
      }
      const receipt = receiptSchema.parse(response.data.data)
      if (receipt.operation_key !== pending.operation_key) {
        throw new AccountClosureFailure('unknown')
      }
      return receipt
    }, false)
  } catch (error) {
    current(scope)
    throw failure(error, true)
  }
}
export async function readClosureReceipt(pending: AccountClosurePending) {
  pendingSchema.parse(pending)
  try {
    const response = await api.get(
      `/api/user/self/closure/receipts/${pending.operation_key}`,
      {
        skipAuthRefresh: true,
        skipErrorHandler: true,
        skipBusinessError: true,
        disableDuplicate: true,
        timeout: 10000,
      }
    )
    if (response.data?.success !== true) {
      throw new AccountClosureFailure('unknown')
    }
    const receipt = receiptSchema.parse(response.data.data)
    if (receipt.operation_key !== pending.operation_key) {
      throw new AccountClosureFailure('unknown')
    }
    return receipt
  } catch (error) {
    throw failure(error, false)
  }
}
