/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import { api } from '@/lib/api'
import { applyAuthBundle } from '@/lib/auth-session'
import { useAuthStore } from '@/stores/auth-store'
import { installSupportedAuthBrowser } from '@/test-support/supported-auth-browser'

import { saveProfileDisplayName, validProfileDisplayName } from './identity-api'
import { profileSearchSchema } from './navigation'

installSupportedAuthBrowser()
const adapter = api.defaults.adapter
const previousAuth = useAuthStore.getState().auth
const scope = { userId: 501, sessionId: 'synthetic-profile-session' }
function login(sid = scope.sessionId) {
  applyAuthBundle(
    {
      access_token: 'synthetic-profile-token',
      token_type: 'Bearer',
      access_expires_at: 2000000000,
      user: { id: scope.userId, role: 1, username: 'synthetic-profile' },
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
    },
    false
  )
}
afterEach(() => {
  api.defaults.adapter = adapter
  useAuthStore.setState({ auth: previousAuth })
})

test('profile tab links accept known destinations and recover unknown tabs to overview', () => {
  for (const tab of [
    'overview',
    'account',
    'security',
    'teams',
    'preferences',
    'rewards',
  ]) {
    assert.equal(profileSearchSchema.parse({ tab }).tab, tab)
  }
  assert.equal(
    profileSearchSchema.parse({ tab: 'billing-admin' }).tab,
    undefined
  )
  assert.equal(profileSearchSchema.parse({}).tab, undefined)
})

test('display names follow the self-profile byte limit and reject controls', () => {
  assert.ok(validProfileDisplayName(''))
  assert.ok(validProfileDisplayName('界'.repeat(85)))
  assert.ok(validProfileDisplayName('a'.repeat(256)))
  assert.equal(validProfileDisplayName('界'.repeat(86)), false)
  assert.equal(validProfileDisplayName('a'.repeat(257)), false)
  assert.equal(validProfileDisplayName('name\n'), false)
  assert.equal(validProfileDisplayName('name\u0085'), false)
})

test('display name save writes only the name and confirms the same identity by reading self', async () => {
  login()
  const calls: string[] = []
  api.defaults.adapter = async (config) => {
    calls.push(config.method ?? '')
    assert.equal(config.url, '/api/user/self')
    assert.deepEqual(config.authScope, scope)
    if (config.method === 'put') {
      assert.deepEqual(JSON.parse(config.data), { display_name: 'New name' })
    }
    return {
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data:
        config.method === 'put'
          ? { success: true }
          : {
              success: true,
              data: {
                id: scope.userId,
                display_name: 'New name',
                username: 'synthetic-profile',
              },
            },
    }
  }
  const result = await saveProfileDisplayName(scope, 'New name')
  assert.equal(result.display_name, 'New name')
  assert.deepEqual(calls, ['put', 'get'])
})

test('a display name write cannot continue after the browser switches sessions', async () => {
  login()
  let calls = 0
  api.defaults.adapter = async (config) => {
    calls++
    login('another-profile-session')
    return {
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: { success: true },
    }
  }
  await assert.rejects(saveProfileDisplayName(scope, 'New name'))
  await assert.rejects(saveProfileDisplayName(scope, 'New name'))
  assert.equal(calls, 1)
})

test('rejected writes and mismatched readback never report a confirmed display name', async () => {
  login()
  let rejectWrite = true
  let reads = 0
  api.defaults.adapter = async (config) => {
    if (config.method === 'get') reads++
    return {
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data:
        config.method === 'put'
          ? { success: !rejectWrite }
          : {
              success: true,
              data: { id: scope.userId, display_name: 'Old name' },
            },
    }
  }
  await assert.rejects(
    saveProfileDisplayName(scope, 'New name'),
    /Profile update failed/
  )
  assert.equal(reads, 0)
  rejectWrite = false
  await assert.rejects(
    saveProfileDisplayName(scope, 'New name'),
    /could not be confirmed/
  )
  assert.equal(reads, 1)
})
