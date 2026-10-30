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
export const securitySnapshotSchema = z
  .object({
    contract: z.literal('core.admin-user-security.v1'),
    user_id: id,
    auth_version: id,
    active: z.boolean(),
    account_active: z.boolean(),
    mfa_enabled: z.boolean(),
  })
  .strict()
export type SecuritySnapshot = z.infer<typeof securitySnapshotSchema>
export type SecurityAction =
  | 'disable'
  | 'enable'
  | 'reset_mfa'
  | 'revoke_credentials'

function current(scope: NativeUsersScope, signal?: AbortSignal) {
  if (!isCurrentNativeUsersScope(scope) || signal?.aborted) {
    throw new NativeProfileFailure('session_changed')
  }
}
function failed(error: unknown, write: boolean): NativeProfileFailure {
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
    status === 401 || status === 403
      ? 'forbidden'
      : status === 409
        ? 'conflict'
        : status === 400
          ? 'invalid'
          : write
            ? 'unknown'
            : 'unavailable'
  )
}
function path(userId: string) {
  if (!id.safeParse(userId).success) throw new NativeProfileFailure('invalid')
  return `/core/v1/admin/user-security/${userId}`
}

export async function readUserSecurity(
  scope: NativeUsersScope,
  userId: string,
  signal?: AbortSignal
): Promise<SecuritySnapshot> {
  current(scope, signal)
  try {
    const response = await api.get<unknown>(
      path(userId),
      nativeUsersOptions(scope, signal)
    )
    current(scope, signal)
    const parsed = securitySnapshotSchema.safeParse(response.data)
    if (!parsed.success || parsed.data.user_id !== userId) {
      throw new NativeProfileFailure('unavailable')
    }
    return parsed.data
  } catch (error) {
    current(scope, signal)
    throw failed(error, false)
  }
}

export async function manageUserSecurity(
  scope: NativeUsersScope,
  before: SecuritySnapshot,
  action: SecurityAction,
  reason: string
) {
  current(scope)
  if (
    !reason.trim() ||
    new TextEncoder().encode(reason).length > 1000 ||
    Array.from(reason).some((character) => {
      const code = character.charCodeAt(0)
      return code <= 31 || (code >= 127 && code <= 159)
    })
  ) {
    throw new NativeProfileFailure('invalid')
  }
  try {
    const response = await api.post<unknown>(
      `${path(before.user_id)}/actions`,
      {
        expected_auth_version: before.auth_version,
        action,
        reason,
      },
      nativeUsersOptions(scope)
    )
    current(scope)
    const parsed = z
      .object({ changed: z.boolean(), user: securitySnapshotSchema })
      .strict()
      .safeParse(response.data)
    if (!parsed.success || parsed.data.user.user_id !== before.user_id) {
      throw new NativeProfileFailure('unknown')
    }
    return parsed.data
  } catch (error) {
    current(scope)
    throw failed(error, true)
  }
}
