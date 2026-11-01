/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

import { readNativeUsersScope } from './access'
import {
  manageUserSecurity,
  readUserSecurity,
  type SecuritySnapshot,
} from './security-api'

const adapter = api.defaults.adapter
const previousAuth = useAuthStore.getState().auth
const user: SecuritySnapshot = {
  contract: 'core.admin-user-security.v1',
  user_id: '9007199254740993',
  auth_version: '9007199254740997',
  active: true,
  account_active: true,
  mfa_enabled: true,
}
function login(sid = 'synthetic-security-session') {
  useAuthStore.getState().auth.setBundle({
    access_token: 'synthetic-security-bearer',
    token_type: 'Bearer',
    access_expires_at: 2000000000,
    user: { id: 501, role: 10, username: 'synthetic-admin' },
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
  const scope = readNativeUsersScope()
  assert.ok(scope)
  return scope
}
afterEach(() => {
  api.defaults.adapter = adapter
  useAuthStore.setState({ auth: previousAuth })
})

test('security mutation keeps exact 64-bit identity generation and scoped bearer with one write', async () => {
  const scope = login()
  let calls = 0
  api.defaults.adapter = async (config) => {
    calls++
    assert.equal(
      config.url,
      `/core/v1/admin/user-security/${user.user_id}/actions`
    )
    assert.deepEqual(JSON.parse(String(config.data)), {
      expected_auth_version: user.auth_version,
      action: 'disable',
      reason: 'Reviewed abuse report',
    })
    assert.equal(config.skipAuthRefresh, true)
    assert.deepEqual(config.authScope, {
      userId: scope.userId,
      sessionId: scope.sessionId,
    })
    return {
      config,
      data: {
        changed: true,
        user: { ...user, active: false, auth_version: '9007199254740998' },
      },
      headers: {},
      status: 200,
      statusText: 'OK',
    }
  }
  assert.equal(
    (await manageUserSecurity(scope, user, 'disable', 'Reviewed abuse report'))
      .user.active,
    false
  )
  assert.equal(calls, 1)
})

test('security reads cannot turn mismatched identities or imprecise versions into editable state', async () => {
  const scope = login()
  for (const data of [
    { ...user, user_id: '1' },
    { ...user, auth_version: 9007199254740996 },
    { success: true, data: user },
  ]) {
    api.defaults.adapter = async (config) => ({
      config,
      data,
      headers: {},
      status: 200,
      statusText: 'OK',
    })
    await assert.rejects(readUserSecurity(scope, user.user_id), /unavailable/)
  }
})

test('security stale generation, denied mutation, and transport uncertainty never replay', async () => {
  const scope = login()
  for (const [status, kind] of [
    [409, 'conflict'],
    [403, 'forbidden'],
    [503, 'unknown'],
  ] as const) {
    let calls = 0
    api.defaults.adapter = async () => {
      calls++
      throw Object.assign(new Error('synthetic failure'), {
        response: { status },
      })
    }
    await assert.rejects(
      manageUserSecurity(scope, user, 'reset_mfa', 'Account recovery verified'),
      new RegExp(kind)
    )
    assert.equal(calls, 1)
  }
})

test('invalid reason and changed Session stop dispatch; late responses lose their scope', async () => {
  const scope = login()
  let calls = 0
  api.defaults.adapter = async (config) => {
    calls++
    login('replacement-security-session')
    return {
      config,
      data: { changed: true, user },
      headers: {},
      status: 200,
      statusText: 'OK',
    }
  }
  await assert.rejects(
    manageUserSecurity(scope, user, 'disable', ' '),
    /invalid/
  )
  await assert.rejects(
    manageUserSecurity(scope, user, 'disable', 'reason\ncontrol'),
    /invalid/
  )
  assert.equal(calls, 0)
  await assert.rejects(
    manageUserSecurity(scope, user, 'disable', 'Reviewed request'),
    /session_changed/
  )
  await assert.rejects(
    manageUserSecurity(scope, user, 'disable', 'Reviewed request'),
    /session_changed/
  )
  assert.equal(calls, 1)
})

test('disable sends a separately supplied public reason without exposing the internal reason as public', async () => {
  const scope = login()
  api.defaults.adapter = async (config) => {
    assert.deepEqual(JSON.parse(String(config.data)), {
      expected_auth_version: user.auth_version,
      action: 'disable',
      reason: 'Private moderation evidence',
      public_reason: 'Published restriction explanation',
    })
    return {
      config,
      data: { changed: true, user: { ...user, active: false } },
      headers: {},
      status: 200,
      statusText: 'OK',
    }
  }
  await manageUserSecurity(
    scope,
    user,
    'disable',
    'Private moderation evidence',
    'Published restriction explanation'
  )
})

test('public reason rejects blank, controls, oversized UTF-8, and non-disable actions before dispatch', async () => {
  const scope = login()
  let calls = 0
  api.defaults.adapter = async () => {
    calls++
    throw new Error('must not dispatch')
  }
  for (const publicReason of [
    '',
    '   ',
    'private\ncontrol',
    '界'.repeat(334),
  ]) {
    await assert.rejects(
      manageUserSecurity(
        scope,
        user,
        'disable',
        'Internal reason',
        publicReason
      ),
      /invalid/
    )
  }
  for (const action of ['enable', 'reset_mfa', 'revoke_credentials'] as const) {
    await assert.rejects(
      manageUserSecurity(
        scope,
        user,
        action,
        'Internal reason',
        'Public reason'
      ),
      /invalid/
    )
  }
  await assert.rejects(
    manageUserSecurity(scope, user, 'disable', '', 'Public reason'),
    /invalid/
  )
  assert.equal(calls, 0)
})
