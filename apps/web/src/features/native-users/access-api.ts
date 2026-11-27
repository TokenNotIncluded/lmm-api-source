/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { api } from '@/lib/api'

import {
  isCurrentNativeUsersScope,
  nativeUsersOptions,
  type NativeUsersScope,
} from './access'
import {
  accessValues,
  groupOptionsSchema,
  matchesAccessUpdate,
  userAccessSnapshotSchema,
  userAccessUpdateSchema,
  userAccessValuesSchema,
  validAccessReason,
  type UserAccessSnapshot,
  type UserAccessUpdate,
  type UserAccessValues,
} from './access-contract'
import { NativeProfileFailure } from './profile-api'
import { nativeProfileIdSchema } from './profile-contract'

function current(scope: NativeUsersScope, signal?: AbortSignal) {
  if (!isCurrentNativeUsersScope(scope) || signal?.aborted) {
    throw new NativeProfileFailure('session_changed')
  }
}

function failure(error: unknown, write: boolean): NativeProfileFailure {
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

function path(userId: string) {
  if (!nativeProfileIdSchema.safeParse(userId).success) {
    throw new NativeProfileFailure('invalid')
  }
  return `/core/v1/admin/user-access/${userId}`
}

export async function readUserAccess(
  scope: NativeUsersScope,
  userId: string,
  signal?: AbortSignal
): Promise<UserAccessSnapshot> {
  current(scope, signal)
  const url = path(userId)
  try {
    const response = await api.get<unknown>(
      url,
      nativeUsersOptions(scope, signal)
    )
    current(scope, signal)
    const parsed = userAccessSnapshotSchema.safeParse(response.data)
    if (!parsed.success || parsed.data.user_id !== userId) {
      throw new NativeProfileFailure('unavailable')
    }
    return parsed.data
  } catch (error) {
    current(scope, signal)
    throw failure(error, false)
  }
}

export async function readUserGroupOptions(
  scope: NativeUsersScope,
  userId: string,
  signal: AbortSignal
) {
  current(scope, signal)
  try {
    const response = await api.get<unknown>(
      `${path(userId)}/group-options`,
      nativeUsersOptions(scope, signal)
    )
    current(scope, signal)
    const parsed = groupOptionsSchema.safeParse(response.data)
    if (!parsed.success || parsed.data.access.user_id !== userId) {
      throw new NativeProfileFailure('unavailable')
    }
    return parsed.data
  } catch (error) {
    current(scope, signal)
    throw failure(error, false)
  }
}

export async function updateUserAccess(
  scope: NativeUsersScope,
  before: UserAccessSnapshot,
  values: UserAccessValues,
  reason: string
): Promise<UserAccessUpdate> {
  current(scope)
  if (
    !userAccessSnapshotSchema.safeParse(before).success ||
    before.auth_version === '9223372036854775807' ||
    !userAccessValuesSchema.safeParse(values).success ||
    !validAccessReason(reason)
  ) {
    throw new NativeProfileFailure('invalid')
  }
  try {
    const response = await api.patch<unknown>(
      path(before.user_id),
      {
        expected_auth_version: before.auth_version,
        expected: accessValues(before),
        ...values,
        reason,
      },
      nativeUsersOptions(scope)
    )
    current(scope)
    const parsed = userAccessUpdateSchema.safeParse(response.data)
    if (!parsed.success || !matchesAccessUpdate(parsed.data, before, values)) {
      throw new NativeProfileFailure('unknown')
    }
    return parsed.data
  } catch (error) {
    current(scope)
    throw failure(error, true)
  }
}
