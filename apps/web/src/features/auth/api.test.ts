/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.

For commercial licensing, please contact support@quantumnous.com
*/
import assert from 'node:assert/strict'
import { describe, test } from 'node:test'

import axios, { AxiosError } from 'axios'
import { toast } from 'sonner'

import { api } from '@/lib/api'
import { AUTH_COOKIE_TIMEOUT_MS } from '@/lib/auth-cookie'
import {
  clearAuthentication,
  setDevelopmentAuthRefreshAdapter,
} from '@/lib/auth-session'
import { useAuthStore, type AuthBundle } from '@/stores/auth-store'
import { installSupportedAuthBrowser } from '@/test-support/supported-auth-browser'

import { executeLogout, login } from './api'
import { beginFederatedLogin } from './federated-api'

// Normal supported browser; install after this fixture replaces its globals.
installSupportedAuthBrowser()

describe('password login request errors', () => {
  test('keeps Cookie and session semantics while leaving HTTP and business notifications to the form', async () => {
    const originalAdapter = api.defaults.adapter
    const originalToast = toast.error
    const notifications: unknown[] = []
    let refreshes = 0
    let requests = 0
    const account: AuthBundle = {
      access_token: 'synthetic-existing-token',
      token_type: 'Bearer',
      access_expires_at: Math.floor(Date.now() / 1000) + 600,
      user: { id: 42, username: 'synthetic', role: 1 },
      session: {
        sid: 'synthetic-existing-session',
        current: true,
        login_method: 'password',
        ip: '',
        user_agent: '',
        created_at: 1,
        last_active_at: 1,
        expires_at: 2_000_000_000,
      },
    }
    clearAuthentication(false, 'idle')
    useAuthStore.getState().auth.setBundle(account)
    toast.error = (message) => {
      notifications.push(message)
      return 'password-login-test'
    }
    setDevelopmentAuthRefreshAdapter(async () => {
      refreshes++
      throw new Error('Password login must not refresh or replay')
    })

    try {
      for (const status of [401, 403, 429, 503, 200]) {
        const payload = {
          success: false,
          code: status === 401 ? 'AUTH_UNAUTHORIZED' : 'UNKNOWN_FAILURE',
          message: 'private backend diagnostic must not appear in a toast',
        }
        api.defaults.adapter = async (config) => {
          requests++
          assert.equal(config.method, 'post')
          assert.equal(config.url, '/api/user/login?turnstile=synthetic-proof')
          assert.deepEqual(JSON.parse(config.data), {
            username: 'synthetic',
            password: 'synthetic-password',
          })
          assert.equal(config.withCredentials, true)
          assert.equal(config.authMode, undefined)
          assert.equal(
            config.headers.get('Authorization'),
            'Bearer synthetic-existing-token'
          )
          assert.equal(config.skipAuthRefresh, true)
          assert.equal(config.skipErrorHandler, true)
          assert.equal(config.skipBusinessError, true)
          assert.equal(config.timeout, AUTH_COOKIE_TIMEOUT_MS)
          assert.equal(
            config.signal,
            undefined,
            'an active Cookie writer must not be aborted by the form'
          )
          const response = {
            config,
            status,
            statusText: 'synthetic response',
            headers: {},
            data: payload,
          }
          if (status >= 400) {
            throw new AxiosError(
              'private transport diagnostic',
              'ERR_BAD_RESPONSE',
              config,
              undefined,
              response
            )
          }
          return response
        }
        const attempt = login(
          {
            username: 'synthetic',
            password: 'synthetic-password',
            turnstile: 'synthetic-proof',
          },
          new AbortController().signal
        )
        if (status === 200) {
          assert.deepEqual(await attempt, payload)
        } else {
          await assert.rejects(
            attempt,
            (error: unknown) =>
              error instanceof AxiosError && error.response?.status === status
          )
        }
        assert.deepEqual(notifications, [])
        assert.equal(
          useAuthStore.getState().auth.accessToken,
          account.access_token
        )
        assert.equal(
          useAuthStore.getState().auth.session?.sid,
          account.session.sid
        )
      }
      assert.equal(
        requests,
        5,
        'failures must never replay the password request'
      )
      assert.equal(refreshes, 0)
    } finally {
      api.defaults.adapter = originalAdapter
      setDevelopmentAuthRefreshAdapter(axios.getAdapter(originalAdapter))
      toast.error = originalToast
      clearAuthentication(false, 'idle')
    }
  })

  test('an incomplete MFA response keeps an allowlisted error code', async () => {
    const originalAdapter = api.defaults.adapter
    clearAuthentication(false, 'idle')
    api.defaults.adapter = async (config) => ({
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: { success: true, data: { require_2fa: true } },
    })
    try {
      await assert.rejects(
        login({ username: 'synthetic', password: 'synthetic-password' }),
        (error: unknown) =>
          error instanceof Error &&
          'code' in error &&
          error.code === 'MFA_FLOW_EXPIRED'
      )
      assert.equal(useAuthStore.getState().auth.pending2FAFlowToken, null)
      assert.equal(useAuthStore.getState().auth.user, null)
    } finally {
      api.defaults.adapter = originalAdapter
      clearAuthentication(false, 'idle')
    }
  })
})

function mismatchError() {
  return {
    isAxiosError: true,
    response: {
      status: 409,
      data: { code: 'AUTH_SESSION_MISMATCH' },
    },
  }
}

describe('logout coordination', () => {
  test('returns an unsuccessful response without pretending to sign out', async () => {
    assert.deepEqual(
      await executeLogout({
        getExpectedSID: () => 'session-a',
        request: async () => ({ success: false, message: 'not revoked' }),
      }),
      { success: false, message: 'not revoked' }
    )
  })

  test('a cookie mismatch is rejected without hidden refresh or logout replay', async () => {
    const requestedSIDs: Array<string | undefined> = []
    const error = mismatchError()
    await assert.rejects(
      executeLogout({
        getExpectedSID: () => 'session-a',
        request: async (sid) => {
          requestedSIDs.push(sid)
          throw error
        },
      }),
      (actual) => actual === error
    )
    assert.deepEqual(requestedSIDs, ['session-a'])
  })

  test('treats an already anonymous or legacy 404 logout endpoint as signed out', async () => {
    for (const status of [401, 404]) {
      const result = await executeLogout({
        getExpectedSID: () => 'session-a',
        request: async () => {
          throw {
            isAxiosError: true,
            response: { status, data: { code: 'AUTH_NOT_FOUND' } },
          }
        },
      })
      assert.deepEqual(result, { success: true, message: '' })
    }
  })
})

describe('OAuth flow initialization', () => {
  test('carries the stored invitation into the native browser-bound WeChat ceremony', async () => {
    const originalPost = api.post
    const originalWindow = Object.getOwnPropertyDescriptor(globalThis, 'window')
    const data = new Map<string, string>([['aff', 'inviter-code']])
    const storage = {
      getItem: (key: string) => data.get(key) ?? null,
      setItem: (key: string, value: string) => {
        data.set(key, value)
      },
      removeItem: (key: string) => {
        data.delete(key)
      },
    }
    const provider = {
      id: '43',
      kind: 'wechat' as const,
      slug: 'configured-wechat',
      name: 'WeChat',
      icon: '',
      revision: '1',
      client_id: '',
      authorization_endpoint: '',
      scopes: '',
      qr_code_url: '',
      native_federated: true as const,
    }
    const flow = {
      state: `lmmfe_${'a'.repeat(64)}`,
      expires_at: 2000000000,
      authorization_url: '',
    }
    const requests: unknown[] = []
    Object.defineProperty(globalThis, 'window', {
      configurable: true,
      value: {
        localStorage: storage,
        sessionStorage: storage,
        location: { origin: 'https://web.example.test' },
      },
    })
    api.post = (async (
      url: string,
      body: unknown,
      config: { authMode?: string; skipAuthRefresh?: boolean }
    ) => {
      assert.equal(url, '/api/oauth/federated/43/begin')
      assert.equal(config.authMode, 'anonymous')
      assert.equal(config.skipAuthRefresh, true)
      requests.push(body)
      return { data: { success: true, data: flow } }
    }) as typeof api.post
    try {
      assert.deepEqual(await beginFederatedLogin(provider, true), flow)
      data.delete('aff')
      await beginFederatedLogin(provider, false)
      assert.deepEqual(requests, [
        { accepted_legal: true, affiliate: 'inviter-code' },
        { accepted_legal: false, affiliate: '' },
      ])
    } finally {
      api.post = originalPost
      if (originalWindow) {
        Object.defineProperty(globalThis, 'window', originalWindow)
      } else Reflect.deleteProperty(globalThis, 'window')
    }
  })
})
