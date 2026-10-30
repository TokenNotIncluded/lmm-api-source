/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import { api } from '@/lib/api'
import { capturePasswordLoginGuard } from '@/lib/auth-session'
import { useAuthStore, type AuthBundle } from '@/stores/auth-store'

import {
  beginPasskeyLogin,
  capturePasskeySession,
  finishPasskeyLogin,
  finishPasskeyRegistration,
  getPasskeyStatus,
} from './api'

const previousAuth = useAuthStore.getState().auth
const adapter = api.defaults.adapter
const flow = `lmmp_${'a'.repeat(64)}`
function bundle(
  sid = 'synthetic-passkey-family',
  token = 'synthetic-passkey-access'
): AuthBundle {
  return {
    access_token: token,
    token_type: 'Bearer',
    access_expires_at: 2000000000,
    user: { id: 501, username: 'synthetic-passkey-user', role: 1 },
    session: {
      sid,
      current: true,
      login_method: 'passkey',
      ip: '127.0.0.1',
      user_agent: 'synthetic',
      created_at: 1,
      last_active_at: 1,
      expires_at: 2000000000,
    },
  }
}
afterEach(() => {
  api.defaults.adapter = adapter
  useAuthStore.setState({ auth: previousAuth })
})

test('Passkey registration scopes its write and commits the returned security rotation once', async () => {
  useAuthStore.getState().auth.setBundle(bundle())
  const scope = capturePasskeySession()
  let calls = 0
  api.defaults.adapter = async (config) => {
    calls++
    assert.equal(config.url, '/api/user/passkey/register/finish')
    assert.equal(config.skipAuthRefresh, true)
    assert.deepEqual(config.authScope, scope)
    assert.deepEqual(JSON.parse(String(config.data)), {
      flow_token: flow,
      credential: { id: 'synthetic' },
      name: 'Laptop',
    })
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: {
        success: true,
        data: bundle(undefined, 'rotated-synthetic-access'),
      },
    }
  }
  await finishPasskeyRegistration(flow, { id: 'synthetic' }, 'Laptop', scope)
  assert.equal(calls, 1)
  assert.equal(
    useAuthStore.getState().auth.accessToken,
    'rotated-synthetic-access'
  )
  useAuthStore.getState().auth.setBundle(bundle('new-family'))
  await assert.rejects(
    finishPasskeyRegistration(flow, {}, 'Laptop', scope),
    /Session expired/
  )
  assert.equal(calls, 1)
})

test('Passkey login carries the optional username and continues the existing MFA ceremony', async () => {
  useAuthStore.getState().auth.reset()
  const assertCurrent = capturePasswordLoginGuard()
  const mfa = {
    require_2fa: true,
    flow_token: `lmmf_${'b'.repeat(64)}`,
    expires_at: 2000000000,
  }
  let calls = 0
  api.defaults.adapter = async (config) => {
    calls++
    assert.equal(config.skipAuthRefresh, true)
    const data = config.url?.endsWith('/begin')
      ? { flow_token: flow, options: { publicKey: {} }, expires_at: 2000000000 }
      : mfa
    if (config.url?.endsWith('/begin')) {
      assert.deepEqual(JSON.parse(String(config.data)), { username: 'alice' })
    }
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: { success: true, data },
    }
  }
  assert.equal(
    (await beginPasskeyLogin(' alice ', assertCurrent)).data?.flow_token,
    flow
  )
  await finishPasskeyLogin(flow, { id: 'synthetic' }, assertCurrent)
  assert.equal(calls, 2)
  assert.equal(useAuthStore.getState().auth.pending2FAFlowToken, mfa.flow_token)
  assert.equal(useAuthStore.getState().auth.user, null)
})

test('Passkey responses cannot overwrite a superseding login or coerce imprecise credential IDs', async () => {
  useAuthStore.getState().auth.reset()
  const assertCurrent = capturePasswordLoginGuard()
  let calls = 0
  api.defaults.adapter = async (config) => {
    calls++
    useAuthStore.getState().auth.setBundle(bundle('replacement-family'))
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: { success: true, data: bundle('old-family') },
    }
  }
  await assert.rejects(
    finishPasskeyLogin(flow, {}, assertCurrent),
    /superseded/
  )
  assert.equal(useAuthStore.getState().auth.session?.sid, 'replacement-family')
  assert.equal(calls, 1)
  api.defaults.adapter = async (config) => ({
    config,
    headers: {},
    status: 200,
    statusText: 'OK',
    data: {
      success: true,
      data: {
        enabled: true,
        management: 'core_recent_session',
        credentials: [
          {
            id: 9007199254740992,
            name: 'Imprecise fixture',
            created_at: '2026-10-30T00:00:00Z',
            last_used_at: null,
          },
        ],
      },
    },
  })
  await assert.rejects(getPasskeyStatus())
})
