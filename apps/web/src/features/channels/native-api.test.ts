/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

import {
  assertNativeIntegers,
  nativeRequest,
  parseNativeDocument,
  readNativeRoutingScope,
} from './native-api'

const adapter = api.defaults.adapter
const previous = useAuthStore.getState().auth
function login(sid = 'synthetic-channel-administration') {
  useAuthStore.getState().auth.setBundle({
    access_token: 'synthetic-channel-session-token',
    token_type: 'Bearer',
    access_expires_at: 2_000_000_000,
    user: { id: 501, role: 10, username: 'synthetic-admin' },
    session: {
      sid,
      current: true,
      login_method: 'password',
      ip: '127.0.0.1',
      user_agent: 'synthetic',
      created_at: 1,
      last_active_at: 1,
      expires_at: 2_000_000_000,
    },
  })
  const scope = readNativeRoutingScope()
  assert.ok(scope)
  return scope
}
afterEach(() => {
  api.defaults.adapter = adapter
  useAuthStore.setState({ auth: previous })
})

test('native routing rejects rounded identifiers and monetary integers before dispatch', async () => {
  const scope = login()
  let calls = 0
  api.defaults.adapter = async () => {
    calls++
    throw new Error('must not dispatch')
  }
  for (const value of [9_007_199_254_740_992, 1.5, Number.NaN, Infinity]) {
    assert.throws(
      () => assertNativeIntegers({ rules: [{ id: value }] }),
      /safely/
    )
    await assert.rejects(
      nativeRequest(scope, 'PUT', '/core/v1/admin/routing/prices', {
        expected_version: { routing: 1, pricing: 1 },
        rates: { input: value },
      }),
      /safely/
    )
  }
  assert.throws(() => parseNativeDocument('{"id":9007199254740993}'), /safely/)
  assert.throws(() => parseNativeDocument('[]'), /object/)
  assert.equal(calls, 0)
})

test('native publications keep exact version CAS, never refresh/replay and reject malformed success receipts', async () => {
  const scope = login()
  const body = { expected_version: { routing: 4, pricing: 3 }, enabled: false }
  let calls = 0
  api.defaults.adapter = async (config) => {
    calls++
    assert.equal(config.skipAuthRefresh, true)
    assert.deepEqual(config.authScope, {
      userId: 501,
      sessionId: scope.sessionId,
    })
    assert.deepEqual(JSON.parse(String(config.data)), body)
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: { success: true, data: { version: { routing: 5, pricing: 3 } } },
    }
  }
  assert.deepEqual(
    await nativeRequest(scope, 'PUT', '/api/channel/1/status', body),
    { version: { routing: 5, pricing: 3 } }
  )
  assert.equal(calls, 1)
  for (const data of [
    { success: false, message: 'Denied', data: {} },
    { success: true, data: {} },
    { success: true, data: { version: { routing: 4, pricing: 3 } } },
  ]) {
    api.defaults.adapter = async (config) => ({
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data,
    })
    await assert.rejects(
      nativeRequest(scope, 'PUT', '/api/channel/1/status', body)
    )
  }
  calls = 0
  api.defaults.adapter = async () => {
    calls++
    throw new Error('commit acknowledgement lost')
  }
  await assert.rejects(
    nativeRequest(scope, 'PUT', '/api/channel/1/status', body),
    /acknowledgement lost/
  )
  assert.equal(calls, 1)
})

test('a changed administrator session cannot reuse a previous editor or accept a late private response', async () => {
  const scope = login()
  let calls = 0
  api.defaults.adapter = async (config) => {
    calls++
    login('replacement-admin-session')
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: { success: true, data: { private: true } },
    }
  }
  await assert.rejects(
    nativeRequest(scope, 'GET', '/core/v1/admin/routing/configuration')
  )
  assert.equal(calls, 1)
  await assert.rejects(
    nativeRequest(scope, 'PUT', '/api/channel/1/status', {
      expected_version: { routing: 1, pricing: 1 },
      enabled: true,
    })
  )
  assert.equal(calls, 1)
})
