/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import axios from 'axios'

import { api } from '@/lib/api'
import { applyAuthBundle } from '@/lib/auth-session'
import { useAuthStore } from '@/stores/auth-store'
import { installSupportedAuthBrowser } from '@/test-support/supported-auth-browser'

import {
  getIPAllowlist,
  IPAllowlistFailure,
  parseIPAllowlistDraft,
  saveIPAllowlist,
} from './ip-allowlist-api'

installSupportedAuthBrowser()
const adapter = api.defaults.adapter
const previousAuth = useAuthStore.getState().auth
const scope = { userId: 501, sessionId: 'synthetic-ip-allowlist-session' }

function login(sid = scope.sessionId) {
  applyAuthBundle(
    {
      access_token: 'synthetic-ip-allowlist-token',
      token_type: 'Bearer',
      access_expires_at: 2000000000,
      user: { id: scope.userId, role: 1, username: 'synthetic-profile' },
      session: {
        sid,
        current: true,
        login_method: 'password',
        ip: '203.0.113.8',
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

test('IP drafts accept IPv4, IPv6 and networks without changing CIDR semantics', () => {
  assert.deepEqual(
    parseIPAllowlistDraft(
      ' 203.0.113.8,\n2001:db8::/32\n203.0.113.8 \n198.51.100.0/24'
    ),
    ['203.0.113.8', '2001:db8::/32', '198.51.100.0/24']
  )
  assert.deepEqual(parseIPAllowlistDraft(' \n'), [])
})

test('IP settings use the dedicated endpoint and return the server-normalized policy', async () => {
  login()
  const policy = { allow_ips: ['203.0.113.8/32'], client_ip: '203.0.113.8' }
  api.defaults.adapter = async (config) => {
    assert.equal(config.url, '/api/user/self/ip-allowlist')
    assert.equal(config.skipBusinessError, true)
    assert.deepEqual(config.authScope, scope)
    if (config.method === 'put') assert.equal(config.skipAuthRefresh, true)
    if (config.method === 'put') {
      assert.deepEqual(JSON.parse(String(config.data)), {
        allow_ips: ['203.0.113.8'],
      })
    }
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: { success: true, data: policy },
    }
  }
  assert.deepEqual(await getIPAllowlist(scope), policy)
  assert.deepEqual(await saveIPAllowlist(scope, ['203.0.113.8']), policy)
})

test('a lockout rejection keeps the server error code for the actionable form error', async () => {
  login()
  api.defaults.adapter = async (config) => ({
    config,
    headers: {},
    status: 200,
    statusText: 'OK',
    data: { success: false, code: 'USER_IP_ALLOWLIST_LOCKOUT' },
  })
  await assert.rejects(
    saveIPAllowlist(scope, ['198.51.100.8']),
    (error: unknown) =>
      error instanceof IPAllowlistFailure &&
      error.code === 'USER_IP_ALLOWLIST_LOCKOUT'
  )
})

test('stale sessions cannot dispatch or receive another session’s IP rules', async () => {
  login('new-session')
  let calls = 0
  api.defaults.adapter = async (config) => {
    calls++
    login('new-session')
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: {
        success: true,
        data: { allow_ips: [], client_ip: null },
      },
    }
  }
  await assert.rejects(saveIPAllowlist(scope, []), /SESSION_CHANGED/)
  assert.equal(calls, 0)
  login()
  await assert.rejects(getIPAllowlist(scope), (error: unknown) =>
    axios.isCancel(error)
  )
  assert.equal(calls, 1)
  assert.equal(useAuthStore.getState().auth.session?.sid, 'new-session')
})
