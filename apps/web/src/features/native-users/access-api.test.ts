/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

import { readNativeUsersScope } from './access'
import { readUserAccess, updateUserAccess } from './access-api'
import {
  accessValues,
  parseAccessGroups,
  userAccessValuesSchema,
  type UserAccessSnapshot,
} from './access-contract'

const originalAdapter = api.defaults.adapter
const previousAuth = useAuthStore.getState().auth
const before: UserAccessSnapshot = {
  contract: 'core.admin-user-access.v1',
  user_id: '9007199254740993',
  auth_version: '9007199254740997',
  active: true,
  account_active: true,
  service_level: 1,
  developer_access_granted: false,
  groups: [' paid group '],
}
const values = {
  service_level: 2,
  developer_access_granted: true,
  groups: ['paid', '研发'],
}

function login(sid = 'synthetic-access-session') {
  useAuthStore.getState().auth.setBundle({
    access_token: 'synthetic-access-bearer',
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
  api.defaults.adapter = originalAdapter
  useAuthStore.setState({ auth: previousAuth })
})

test('access update binds an exact snapshot and generation, without refreshing or replaying', async () => {
  const scope = login()
  let calls = 0
  api.defaults.adapter = async (config) => {
    calls++
    assert.equal(config.url, `/core/v1/admin/user-access/${before.user_id}`)
    assert.equal(config.method, 'patch')
    assert.equal(config.skipAuthRefresh, true)
    assert.deepEqual(config.authScope, {
      userId: scope.userId,
      sessionId: scope.sessionId,
    })
    assert.deepEqual(JSON.parse(String(config.data)), {
      expected_auth_version: before.auth_version,
      expected: accessValues(before),
      ...values,
      reason: 'Approved developer access',
    })
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: {
        changed: true,
        user: { ...before, ...values, auth_version: '9007199254740998' },
      },
    }
  }
  const result = await updateUserAccess(
    scope,
    before,
    values,
    'Approved developer access'
  )
  assert.equal(result.user.user_id, before.user_id)
  assert.deepEqual(result.user.groups, values.groups)
  assert.equal(calls, 1)
})

test('groups preserve exact names, match C order, and reject duplicate or reserved grants', async () => {
  const groups = parseAccessGroups('研发\n paid group \n\n𐀀\n\ue000\r\npaid')
  assert.deepEqual(groups, [' paid group ', 'paid', '研发', '\ue000', '𐀀'])
  assert.equal(
    userAccessValuesSchema.safeParse({ ...values, groups }).success,
    true
  )
  const scope = login()
  let calls = 0
  api.defaults.adapter = async () => {
    calls++
    throw new Error('must not dispatch')
  }
  for (const groups of [
    ['auto'],
    ['paid', 'paid'],
    ['\n'],
    ['x'.repeat(65)],
    Array.from({ length: 101 }, (_, i) => `g${i.toString().padStart(3, '0')}`),
  ]) {
    await assert.rejects(
      updateUserAccess(scope, before, { ...values, groups }, 'Approved change'),
      /invalid/
    )
  }
  for (const reason of [' ', 'Private\nreason', '界'.repeat(334)]) {
    await assert.rejects(
      updateUserAccess(scope, before, values, reason),
      /invalid/
    )
  }
  assert.equal(calls, 0)
})

test('reads reject mismatched users and writes reject mismatched or stale receipts', async () => {
  const scope = login()
  for (const data of [
    { ...before, user_id: '1' },
    { ...before, auth_version: 9007199254740996 },
  ]) {
    api.defaults.adapter = async (config) => ({
      config,
      data,
      headers: {},
      status: 200,
      statusText: 'OK',
    })
    await assert.rejects(readUserAccess(scope, before.user_id), /unavailable/)
  }
  for (const user of [
    before,
    { ...before, ...values },
    { ...before, ...values, user_id: '2', auth_version: '9007199254740998' },
  ]) {
    api.defaults.adapter = async (config) => ({
      config,
      data: { changed: true, user },
      headers: {},
      status: 200,
      statusText: 'OK',
    })
    await assert.rejects(
      updateUserAccess(scope, before, values, 'Approved change'),
      /unknown/
    )
  }
})

test('revoking several grants accepts the final generation returned by Core triggers', async () => {
  const scope = login()
  api.defaults.adapter = async (config) => ({
    config,
    data: {
      changed: true,
      user: { ...before, ...values, auth_version: '9007199254741001' },
    },
    headers: {},
    status: 200,
    statusText: 'OK',
  })
  assert.equal(
    (await updateUserAccess(scope, before, values, 'Reviewed grants')).user
      .auth_version,
    '9007199254741001'
  )
})

test('conflicts, permission denials and unknown writes are single attempts; stale sessions cannot submit', async () => {
  const scope = login()
  for (const [status, kind] of [
    [409, 'conflict'],
    [403, 'forbidden'],
    [503, 'unknown'],
  ] as const) {
    let calls = 0
    api.defaults.adapter = async () => {
      calls++
      throw Object.assign(new Error('synthetic'), { response: { status } })
    }
    await assert.rejects(
      updateUserAccess(scope, before, values, 'Approved change'),
      new RegExp(kind)
    )
    assert.equal(calls, 1)
  }
  let calls = 0
  api.defaults.adapter = async (config) => {
    calls++
    login('replacement-session')
    return { config, data: before, headers: {}, status: 200, statusText: 'OK' }
  }
  await assert.rejects(readUserAccess(scope, before.user_id), /session_changed/)
  await assert.rejects(
    updateUserAccess(scope, before, values, 'Approved change'),
    /session_changed/
  )
  assert.equal(calls, 1)
})
