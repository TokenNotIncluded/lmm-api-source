/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import { AxiosError, type AxiosAdapter } from 'axios'

import { api } from '@/lib/api'
import { setDevelopmentAuthRefreshAdapter } from '@/lib/auth-session'
import { useAuthStore, type AuthBundle } from '@/stores/auth-store'
import { installSupportedAuthBrowser } from '@/test-support/supported-auth-browser'

import { createTransfer, getTransferBalance, listTransfers } from './api'

installSupportedAuthBrowser()
const originalAdapter = api.defaults.adapter
const originalAuth = useAuthStore.getState().auth
afterEach(() => {
  api.defaults.adapter = originalAdapter
  setDevelopmentAuthRefreshAdapter(originalAdapter as AxiosAdapter)
  useAuthStore.setState({ auth: originalAuth })
})

test('expired wallet reads renew the same session while a financial POST is never replayed', async () => {
  const now = Math.floor(Date.now() / 1000)
  const bundle: AuthBundle = {
    user: { id: 52, username: 'synthetic-transfer', role: 1 },
    access_token: 'expired-transfer-token',
    token_type: 'Bearer',
    access_expires_at: now - 10,
    session: {
      sid: 'synthetic-transfer-session',
      current: true,
      login_method: 'password',
      ip: '127.0.0.1',
      user_agent: 'synthetic-transfer-test',
      created_at: now - 3600,
      last_active_at: now - 300,
      expires_at: now + 86400,
    },
  }
  const scope = { userId: bundle.user.id, sessionId: bundle.session.sid }
  useAuthStore.getState().auth.setBundle(bundle)
  let refreshes = 0
  let writes = 0
  setDevelopmentAuthRefreshAdapter(async (config) => {
    refreshes++
    return {
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: {
        success: true,
        data: {
          ...bundle,
          access_token: 'renewed-transfer-token',
          access_expires_at: now + 600,
        },
      },
    }
  })
  api.defaults.adapter = async (config) => {
    assert.deepEqual(config.authScope, scope)
    assert.equal(
      config.headers.get('Authorization'),
      'Bearer renewed-transfer-token'
    )
    if (config.method === 'post') {
      writes++
      assert.equal(config.skipAuthRefresh, true)
      throw new AxiosError(
        'Unauthorized',
        'ERR_BAD_REQUEST',
        config,
        undefined,
        {
          config,
          status: 401,
          statusText: 'Unauthorized',
          headers: {},
          data: { success: false },
        }
      )
    }
    return {
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: {
        success: true,
        data:
          config.url === '/api/wallet/balance'
            ? {
                schema_version: 2,
                currency_unit: 'CREDIT',
                credits_per_usd: 500000,
                public_credits_per_usd_exact: '500000',
                spendable_credits: 125,
              }
            : [],
      },
    }
  }

  assert.equal((await getTransferBalance(scope)).spendable_credits, 125)
  assert.deepEqual(await listTransfers(scope), [])
  assert.equal(refreshes, 1)
  await assert.rejects(createTransfer(scope, 100, 'synthetic-transfer-request'))
  assert.equal(writes, 1)
  assert.equal(refreshes, 1)
})
