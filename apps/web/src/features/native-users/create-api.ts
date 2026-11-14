/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

import { api } from '@/lib/api'

import {
  isCurrentNativeUsersScope,
  nativeUsersOptions,
  type NativeUsersScope,
} from './access'

export type NativeUserDraft = {
  username: string
  password: string
  display_name: string
}
export function validNativeUserDraft(draft: NativeUserDraft): boolean {
  const username = Array.from(draft.username).length
  const password = Array.from(draft.password).length
  return (
    username >= 1 &&
    username <= 20 &&
    draft.username.trim() === draft.username &&
    !/[\p{Cc}\p{Cf}]/u.test(draft.username) &&
    password >= 8 &&
    password <= 20 &&
    Array.from(draft.display_name).length <= 20 &&
    !/[\p{Cc}\p{Cf}]/u.test(draft.display_name)
  )
}
export class NativeUserCreationFailure extends Error {
  constructor(
    readonly kind:
      | 'invalid'
      | 'conflict'
      | 'forbidden'
      | 'unknown'
      | 'session_changed'
  ) {
    super(`User creation: ${kind}`)
  }
}
const created = z
  .object({
    version: z.literal(1),
    user_id: z.string().regex(/^[1-9][0-9]*$/),
    platform_role: z.literal('user'),
  })
  .strict()
export async function createNativeUser(
  scope: NativeUsersScope,
  draft: NativeUserDraft
): Promise<string> {
  if (!isCurrentNativeUsersScope(scope)) {
    throw new NativeUserCreationFailure('session_changed')
  }
  if (!validNativeUserDraft(draft)) {
    throw new NativeUserCreationFailure('invalid')
  }
  try {
    const response = await api.post<unknown>(
      '/core/v1/admin/users',
      { ...draft },
      nativeUsersOptions(scope)
    )
    if (!isCurrentNativeUsersScope(scope)) {
      throw new NativeUserCreationFailure('session_changed')
    }
    const result = created.safeParse(response.data)
    if (!result.success) throw new NativeUserCreationFailure('unknown')
    return result.data.user_id
  } catch (error) {
    if (!isCurrentNativeUsersScope(scope)) {
      throw new NativeUserCreationFailure('session_changed')
    }
    if (error instanceof NativeUserCreationFailure) throw error
    const status = (error as { response?: { status?: number } })?.response
      ?.status
    throw new NativeUserCreationFailure(
      status === 401 || status === 403
        ? 'forbidden'
        : status === 400
          ? 'invalid'
          : status === 409
            ? 'conflict'
            : 'unknown'
    )
  }
}
