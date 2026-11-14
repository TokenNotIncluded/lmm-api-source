/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
// @ts-expect-error Bun's test module is only available in the test runtime.
import { afterEach, expect, test } from 'bun:test'

import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

import { createNativeUser, validNativeUserDraft } from './create-api'

const originalAdapter = api.defaults.adapter
const previous = useAuthStore.getState().auth
const scope = {
  userId: 7,
  sessionId: 'synthetic-create-admin',
  role: 10 as const,
}
const draft = {
  username: 'new-user',
  password: '密'.repeat(8),
  display_name: '',
}
function login(sid = scope.sessionId) {
  useAuthStore
    .getState()
    .auth.setBundle({
      access_token: 'synthetic-credential',
      token_type: 'Bearer',
      access_expires_at: 2000000000,
      user: { id: 7, role: 10, username: 'admin' },
      session: {
        sid,
        current: true,
        login_method: 'password',
        ip: '127.0.0.1',
        user_agent: 'synthetic',
        created_at: 1,
        last_active_at: 1,
        expires_at: 2000000000,
      },
    })
}
afterEach(() => {
  api.defaults.adapter = originalAdapter
  useAuthStore.setState({ auth: previous })
})
test('native creation sends only ordinary-user fields with current Session and exact response IDs', async () => {
  login()
  api.defaults.adapter = async (config) => {
    expect(config.url).toBe('/core/v1/admin/users')
    expect(config.authScope).toEqual({ userId: 7, sessionId: scope.sessionId })
    expect(config.skipAuthRefresh).toBe(true)
    expect(JSON.parse(config.data)).toEqual(draft)
    return {
      config,
      status: 201,
      statusText: 'Created',
      headers: {},
      data: { version: 1, user_id: '9007199254740993', platform_role: 'user' },
    }
  }
  expect(await createNativeUser(scope, draft)).toBe('9007199254740993')
  expect(validNativeUserDraft({ ...draft, password: '🙂'.repeat(20) })).toBe(
    true
  )
  expect(validNativeUserDraft({ ...draft, password: '🙂'.repeat(21) })).toBe(
    false
  )
})
test('lost response remains unknown without retry; changed Session cannot commit success', async () => {
  login()
  let calls = 0
  api.defaults.adapter = async () => {
    calls++
    throw new Error('lost response')
  }
  await expect(createNativeUser(scope, draft)).rejects.toMatchObject({
    kind: 'unknown',
  })
  expect(calls).toBe(1)
  api.defaults.adapter = async (config) => {
    login('new-session')
    return {
      config,
      status: 201,
      statusText: 'Created',
      headers: {},
      data: { version: 1, user_id: '8', platform_role: 'user' },
    }
  }
  await expect(createNativeUser(scope, draft)).rejects.toMatchObject({
    kind: 'session_changed',
  })
})
