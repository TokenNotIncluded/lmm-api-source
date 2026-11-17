/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import axios from 'axios'

import { api } from '@/lib/api'
import { withAuthenticatedCookieWrite } from '@/lib/auth-session'
import { useAuthStore } from '@/stores/auth-store'

import { getUserProfile } from './api'
import type { UserProfile } from './types'

export type ProfileIdentityScope = { userId: number; sessionId: string }

export function validProfileDisplayName(value: string) {
  return (
    new TextEncoder().encode(value).length <= 256 &&
    ![...value].some((character) => {
      const point = character.codePointAt(0) ?? 0
      return point <= 0x1f || (point >= 0x7f && point <= 0x9f)
    })
  )
}

export function currentProfileIdentity(scope: ProfileIdentityScope) {
  const { auth } = useAuthStore.getState()
  return auth.user?.id === scope.userId && auth.session?.sid === scope.sessionId
}

/** Write and read back the same signed-in identity before updating the page. */
export async function saveProfileDisplayName(
  scope: ProfileIdentityScope,
  displayName: string,
  signal?: AbortSignal
): Promise<UserProfile> {
  const assertCurrent = () => {
    if (signal?.aborted || !currentProfileIdentity(scope)) {
      throw new axios.CanceledError('Profile identity changed')
    }
  }
  assertCurrent()
  if (!validProfileDisplayName(displayName)) {
    throw new Error('Invalid display name')
  }
  await withAuthenticatedCookieWrite(async (config) => {
    assertCurrent()
    const result = await api.put(
      '/api/user/self',
      { display_name: displayName },
      {
        ...config,
        authScope: scope,
        signal,
        skipBusinessError: true,
        skipErrorHandler: true,
      }
    )
    assertCurrent()
    if (result.data?.success !== true) throw new Error('Profile update failed')
  }, false)
  assertCurrent()
  const response = await getUserProfile({
    signal,
    skipBusinessError: true,
    skipErrorHandler: true,
  })
  assertCurrent()
  if (
    !response.success ||
    response.data?.id !== scope.userId ||
    response.data.display_name !== displayName
  ) {
    throw new Error('Profile update could not be confirmed')
  }
  return response.data
}
