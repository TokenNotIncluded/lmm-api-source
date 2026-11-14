/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, beforeEach, test } from 'node:test'

import axios, { type AxiosAdapter, AxiosError } from 'axios'
import { Window } from 'happy-dom'

import { logout } from '@/features/auth/api'
import { api } from '@/lib/api'
import {
  getAuthCookieBlockedState,
  withAuthCookieLock,
} from '@/lib/auth-cookie'
import {
  applyAuthBundle,
  assertLoginResultCurrent,
  clearAuthentication,
  setDevelopmentAuthRefreshAdapter,
} from '@/lib/auth-session'
import { useAuthStore, type AuthBundle } from '@/stores/auth-store'

const dom = new Window({ url: 'https://unit.example.test/' })
const saved = new Map(
  ['window', 'navigator', 'isSecureContext', 'fetch'].map((key) => [
    key,
    Object.getOwnPropertyDescriptor(globalThis, key),
  ])
)
Object.defineProperty(globalThis, 'window', { configurable: true, value: dom })
Object.defineProperty(globalThis, 'navigator', {
  configurable: true,
  value: {
    locks: {
      request: async (
        _name: string,
        _options: unknown,
        run: () => Promise<unknown>
      ) => run(),
    },
  },
})
Object.defineProperty(globalThis, 'isSecureContext', {
  configurable: true,
  value: true,
})
const adapter = api.defaults.adapter
const b: AuthBundle = {
  access_token: 'synthetic-B',
  token_type: 'Bearer',
  access_expires_at: 2000000000,
  user: { id: 71, username: 'B', role: 1 },
  session: {
    sid: 'b'.repeat(64),
    current: true,
    login_method: 'password',
    ip: '',
    user_agent: '',
    created_at: 1,
    last_active_at: 1,
    expires_at: 2000000000,
  },
}
const c: AuthBundle = {
  ...b,
  access_token: 'synthetic-C',
  user: { ...b.user, id: 72 },
  session: { ...b.session, sid: 'c'.repeat(64) },
}
beforeEach(() => {
  clearAuthentication(false, 'idle')
  dom.localStorage.clear()
  globalThis.fetch = (() => {
    throw new Error('Unexpected unit fetch')
  }) as typeof fetch
})
afterEach(() => {
  api.defaults.adapter = adapter
  setDevelopmentAuthRefreshAdapter(adapter as AxiosAdapter)
  clearAuthentication(false, 'idle')
})
after(() => {
  dom.close()
  for (const [key, value] of saved) {
    if (value) Object.defineProperty(globalThis, key, value)
    else Reflect.deleteProperty(globalThis, key)
  }
})
const rejectGuard = () => {
  throw new axios.CanceledError('Original login superseded')
}

test('a confirmed B revocation preserves newer C Cookie/store on expected-SID 409', async () => {
  applyAuthBundle(c, false)
  const methods: string[] = []
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const request = new Request(input, init)
    methods.push(request.method)
    assert.equal(request.headers.get('Authorization'), 'Bearer synthetic-B')
    assert.equal(request.headers.get('X-Auth-Session'), b.session.sid)
    if (request.method === 'DELETE') {
      return Response.json({
        success: true,
        data: { revoked_sid: b.session.sid, current: true },
      })
    }
    return Response.json(
      { success: false, code: 'AUTH_SESSION_MISMATCH' },
      { status: 409 }
    )
  }) as typeof fetch
  await assert.rejects(
    withAuthCookieLock(async (tx) =>
      assertLoginResultCurrent(tx, rejectGuard, { success: true, data: b })
    ),
    /Original login superseded/
  )
  assert.deepEqual(methods, ['DELETE', 'POST'])
  assert.equal(useAuthStore.getState().auth.session?.sid, c.session.sid)
})

test('401 from strict family revocation is never normalized into successful rollback', async () => {
  let requests = 0
  globalThis.fetch = (async () => {
    requests++
    return Response.json({ success: false }, { status: 401 })
  }) as typeof fetch
  await assert.rejects(
    withAuthCookieLock(async (tx) =>
      assertLoginResultCurrent(tx, rejectGuard, { success: true, data: b })
    ),
    (error) => axios.isAxiosError(error) && error.response?.status === 401
  )
  assert.equal(requests, 1)
})

test('a wrong revoked_sid receipt is rejected before Cookie cleanup', async () => {
  let requests = 0
  globalThis.fetch = (async () => {
    requests++
    return Response.json({
      success: true,
      data: { revoked_sid: c.session.sid, current: true },
    })
  }) as typeof fetch
  await assert.rejects(
    withAuthCookieLock(async (tx) =>
      assertLoginResultCurrent(tx, rejectGuard, { success: true, data: b })
    )
  )
  assert.equal(requests, 1)
})

test('expired A without its Cookie cannot be reported revoked through idempotent logout', async () => {
  applyAuthBundle({ ...b, access_expires_at: 1 }, false)
  let refreshes = 0,
    legacyLogouts = 0
  setDevelopmentAuthRefreshAdapter(async (config) => {
    refreshes++
    throw new AxiosError(
      'expired A / no A Cookie',
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
  })
  api.defaults.adapter = async (config) => {
    legacyLogouts++
    return {
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: { success: true },
    }
  }
  await assert.rejects(
    logout(),
    (error) => axios.isAxiosError(error) && error.response?.status === 401
  )
  assert.equal(refreshes, 1)
  assert.equal(legacyLogouts, 0)
  assert.equal(useAuthStore.getState().auth.session?.sid, b.session.sid)
})

test('expired A can refresh only its expected family before explicit revocation', async () => {
  applyAuthBundle({ ...b, access_expires_at: 1 }, false)
  setDevelopmentAuthRefreshAdapter(async (config) => {
    assert.equal(config.headers.get('X-Auth-Session'), b.session.sid)
    return {
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: {
        success: true,
        data: { ...b, access_token: 'fresh-A-bound-token' },
      },
    }
  })
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const request = new Request(input, init)
    assert.equal(request.method, 'POST')
    assert.equal(
      request.headers.get('Authorization'),
      'Bearer fresh-A-bound-token'
    )
    assert.equal(request.headers.get('X-Auth-Session'), b.session.sid)
    return Response.json({
      success: true,
      data: { revoked_sid: b.session.sid, current: true },
    })
  }) as typeof fetch
  api.defaults.adapter = async (config) => ({
    config,
    status: 200,
    statusText: 'OK',
    headers: {},
    data: { success: true },
  })
  assert.equal((await logout()).success, true)
  assert.equal(useAuthStore.getState().auth.user, null)
})

test('Root401: unexpired historical A bypasses current-only DELETE and requires a purpose logout receipt', async () => {
  applyAuthBundle(b, false)
  const calls: string[] = []
  let acknowledged = false
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const request = new Request(input, init)
    calls.push(request.method)
    assert.equal(
      request.headers.get('Authorization'),
      `Bearer ${b.access_token}`
    )
    assert.equal(request.headers.get('X-Auth-Session'), b.session.sid)
    if (request.method === 'DELETE') {
      return Response.json({ success: false }, { status: 401 })
    }
    assert.equal(new URL(request.url).pathname, '/api/user/auth/logout')
    acknowledged = true
    return Response.json({
      success: true,
      data: { revoked_sid: b.session.sid },
    })
  }) as typeof fetch
  api.defaults.adapter = async (config) => {
    assert.equal(acknowledged, true)
    return {
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: { success: true },
    }
  }
  assert.equal((await logout()).success, true)
  assert.deepEqual(calls, ['POST'])
  assert.equal(useAuthStore.getState().auth.user, null)
})

test('original A is revoked before queued Cookie work can replace the selected account', async () => {
  applyAuthBundle(b, false)
  const originalLock = navigator.locks.request
  let release!: () => void
  const waiting = new Promise<void>((resolve) => {
    release = resolve
  })
  navigator.locks.request = (async (_name, _options, run) => {
    await waiting
    return run({ name: _name, mode: 'exclusive' })
  }) as typeof navigator.locks.request
  let acknowledged!: () => void
  const revoked = new Promise<void>((resolve) => {
    acknowledged = resolve
  })
  let calls = 0
  let cookieCleanups = 0
  api.defaults.adapter = async (config) => {
    cookieCleanups++
    return {
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: { success: true },
    }
  }
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const request = new Request(input, init)
    calls++
    assert.equal(
      request.headers.get('Authorization'),
      `Bearer ${b.access_token}`
    )
    assert.equal(request.headers.get('X-Auth-Session'), b.session.sid)
    acknowledged()
    return Response.json({
      success: true,
      data: { revoked_sid: b.session.sid },
    })
  }) as typeof fetch
  const pending = logout()
  void pending.catch(() => {})
  let deadline: ReturnType<typeof setTimeout> | undefined
  try {
    await Promise.race([
      revoked,
      new Promise<never>((_resolve, reject) => {
        deadline = setTimeout(
          () => reject(new Error('A revocation waited for Cookie lock')),
          1000
        )
      }),
    ])
    applyAuthBundle(c, false)
    release()
    await assert.rejects(pending, /Logout superseded/)
    assert.equal(calls, 1)
    assert.equal(cookieCleanups, 0)
    assert.equal(useAuthStore.getState().auth.session?.sid, c.session.sid)
  } finally {
    clearTimeout(deadline)
    release()
    await pending.catch(() => {})
    navigator.locks.request = originalLock
  }
})

test('purpose logout 200 without exact A receipt never confirms revocation', async () => {
  const { revokeLogoutSession } = await import('@/lib/auth-session')
  for (const data of [
    undefined,
    { revoked_sid: null },
    { revoked_sid: c.session.sid },
  ]) {
    globalThis.fetch = (async () =>
      Response.json({ success: true, data })) as typeof fetch
    await assert.rejects(
      revokeLogoutSession({
        userId: b.user.id,
        sid: b.session.sid,
        accessToken: b.access_token,
        accessExpiresAt: b.access_expires_at,
      })
    )
  }
})

test('current login result validation remains synchronous before bundle commit', async () => {
  await withAuthCookieLock(async (tx) => {
    assert.equal(
      assertLoginResultCurrent(tx, () => {}, { success: true, data: b }),
      undefined
    )
  })
})

// Last: deliberately retains the real module's uncertain marker for this process.
test('rollback transport failure retains the existing Cookie uncertainty boundary', async () => {
  applyAuthBundle(c, false)
  globalThis.fetch = (async () => {
    throw new TypeError('fetch failed')
  }) as typeof fetch
  await assert.rejects(
    withAuthCookieLock(async (tx) =>
      assertLoginResultCurrent(tx, rejectGuard, { success: true, data: b })
    )
  )
  assert.equal(getAuthCookieBlockedState(), 'uncertain')
  assert.ok(
    dom.localStorage
      .getItem('new-api:auth-cookie:uncertain')
      ?.startsWith('uncertain:')
  )
  let originalRevocations = 0
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const request = new Request(input, init)
    assert.equal(request.headers.get('X-Auth-Session'), c.session.sid)
    originalRevocations++
    return Response.json({
      success: true,
      data: { revoked_sid: c.session.sid },
    })
  }) as typeof fetch
  await assert.rejects(
    logout(),
    (error: unknown) =>
      error instanceof Error &&
      'code' in error &&
      error.code === 'AUTH_COOKIE_BLOCKED'
  )
  assert.equal(originalRevocations, 1)
  assert.equal(getAuthCookieBlockedState(), 'uncertain')
  // Missing live proof still cannot bypass the Cookie boundary to refresh A.
  useAuthStore.getState().auth.setBundle({ ...c, access_expires_at: 1 })
  await assert.rejects(logout())
  assert.equal(originalRevocations, 1)
})
