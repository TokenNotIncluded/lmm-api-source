/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, beforeEach, test } from 'node:test'

import {
  AxiosError,
  getAdapter,
  isCancel,
  type AxiosAdapter,
  type AxiosResponse,
} from 'axios'
import { toast } from 'sonner'

import {
  AUTH_COOKIE_LOCK,
  AUTH_COOKIE_TIMEOUT_MS,
  withAuthCookieLock,
} from '@/lib/auth-cookie'
import {
  applyAuthBundle,
  clearAuthentication,
  setDevelopmentAuthRefreshAdapter,
} from '@/lib/auth-session'
import { api } from '@/lib/http-client'
import { useAuthStore, type AuthBundle } from '@/stores/auth-store'

import {
  getLoginSessions,
  isCurrentLoginSessionScope,
  revokeLoginSession,
  revokeOtherLoginSessions,
  updateLoginSessionSettings,
  type LoginSessionScope,
} from './api'

const now = 2_000_000_000
const scope: LoginSessionScope = { userId: 42, sessionId: 'session-a' }
const cookieKey = 'new-api:auth-cookie:uncertain'
const originalNow = Date.now
const originalAdapter = api.defaults.adapter
const originalRefreshAdapter = getAdapter(originalAdapter)
const originalToastError = toast.error
const originalToastSuccess = toast.success
const descriptors = new Map(
  ['window', 'navigator', 'isSecureContext'].map((key) => [
    key,
    Object.getOwnPropertyDescriptor(globalThis, key),
  ])
)
const storage = new Map<string, string>()
const toasts: unknown[] = []
const lockNames: string[] = []
let held = 0
let tail: Promise<void> = Promise.resolve()

function deferred() {
  let resolve!: () => void
  const promise = new Promise<void>((done) => {
    resolve = done
  })
  return { promise, resolve }
}

const session: AuthBundle['session'] = {
  sid: scope.sessionId,
  current: true,
  login_method: 'password',
  ip: null,
  user_agent: null,
  created_at: now - 100,
  last_active_at: now,
  expires_at: now + 3600,
}

function bundle(
  token = 'initial',
  userId = scope.userId,
  sessionId = scope.sessionId
): AuthBundle {
  return {
    access_token: token,
    token_type: 'Bearer',
    access_expires_at: now + 600,
    user: { id: userId, username: 'synthetic', role: 1 },
    session: { ...session, sid: sessionId, ip: '', user_agent: '' },
  }
}

function response(
  config: Parameters<AxiosAdapter>[0],
  status: number,
  data: unknown
): AxiosResponse {
  return { config, status, statusText: 'fixture', headers: {}, data }
}

Object.defineProperty(globalThis, 'isSecureContext', {
  configurable: true,
  value: true,
})
Object.defineProperty(globalThis, 'window', {
  configurable: true,
  value: {
    location: { origin: 'https://console.example.test', pathname: '/profile' },
    localStorage: {
      getItem: (key: string) => storage.get(key) ?? null,
      setItem: (key: string, value: string) => storage.set(key, value),
      removeItem: (key: string) => storage.delete(key),
    },
  },
})
Object.defineProperty(globalThis, 'navigator', {
  configurable: true,
  value: {
    locks: {
      async request<T>(
        name: string,
        options: { mode: string; signal?: AbortSignal },
        callback: () => Promise<T>
      ): Promise<T> {
        assert.equal(options.mode, 'exclusive')
        lockNames.push(name)
        const predecessor = tail
        const release = deferred()
        tail = release.promise
        await predecessor
        try {
          if (options.signal?.aborted) {
            throw new DOMException('Aborted', 'AbortError')
          }
          held++
          try {
            assert.equal(held, 1)
            return await callback()
          } finally {
            held--
          }
        } finally {
          release.resolve()
        }
      },
    },
  },
})

beforeEach(() => {
  Date.now = () => now * 1000
  applyAuthBundle(bundle(), false)
  setDevelopmentAuthRefreshAdapter(async () => {
    throw new Error('Unexpected refresh in session API fixture')
  })
  toast.error = toast.success = (...args: Parameters<typeof toast.error>) => {
    toasts.push(args[0])
    return 'session-api-test'
  }
})

afterEach(async () => {
  try {
    await tail
    assert.equal(held, 0)
    assert.ok(lockNames.every((name) => name === AUTH_COOKIE_LOCK))
    assert.equal(storage.size, 0)
    assert.deepEqual(toasts, [])
  } finally {
    api.defaults.adapter = originalAdapter
    setDevelopmentAuthRefreshAdapter(originalRefreshAdapter)
    toast.error = originalToastError
    toast.success = originalToastSuccess
    Date.now = originalNow
    clearAuthentication(false, 'idle')
    lockNames.length = 0
    toasts.length = 0
  }
})

after(() => {
  for (const [key, descriptor] of descriptors) {
    if (descriptor) Object.defineProperty(globalThis, key, descriptor)
    else Reflect.deleteProperty(globalThis, key)
  }
})

const listReply = {
  success: true,
  data: [session],
  session_auto_logout: false,
}
type Operation = {
  name: string
  run: () => Promise<{ success: boolean }>
  method: string
  url: string
  reply: unknown
  cookie: boolean
}
const operations: [Operation, Operation, Operation, Operation] = [
  {
    name: 'list',
    run: () => getLoginSessions(scope),
    method: 'get',
    url: '/api/user/sessions',
    reply: listReply,
    cookie: false,
  },
  {
    name: 'single revoke',
    run: () => revokeLoginSession('other-session', scope),
    method: 'delete',
    url: '/api/user/sessions/other-session',
    reply: {
      success: true,
      data: { revoked_sid: 'other-session', current: false },
    },
    cookie: true,
  },
  {
    name: 'revoke others',
    run: () => revokeOtherLoginSessions(scope),
    method: 'post',
    url: '/api/user/sessions/revoke-others',
    reply: { success: true, data: { revoked_count: 0 } },
    cookie: false,
  },
  {
    name: 'settings',
    run: () => updateLoginSessionSettings(false, scope),
    method: 'put',
    url: '/api/user/sessions/settings',
    reply: { success: true },
    cookie: false,
  },
]
const writes = operations.filter((operation) => operation.method !== 'get')

for (const operation of operations) {
  test(`${operation.name} preserves scope, transport policy and Cookie intent`, async () => {
    let calls = 0
    api.defaults.adapter = async (config) => {
      calls++
      assert.equal(config.method, operation.method)
      assert.equal(config.url, operation.url)
      assert.deepEqual(config.authScope, scope)
      assert.equal(config.headers.get('Authorization'), 'Bearer initial')
      assert.equal(config.skipBusinessError, true)
      assert.equal(config.skipErrorHandler, true)
      if (operation.method === 'get') {
        assert.notEqual(config.skipAuthRefresh, true)
        assert.equal(held, 0)
      } else {
        assert.equal(config.headers.get('X-Auth-Session'), scope.sessionId)
        assert.equal(config.skipAuthRefresh, true)
        assert.equal(config.timeout, AUTH_COOKIE_TIMEOUT_MS)
        assert.equal(held, 1)
      }
      if (operation.cookie) {
        assert.match(storage.get(cookieKey) ?? '', /^pending:/)
      } else {
        assert.equal(storage.get(cookieKey), undefined)
      }
      if (operation.method === 'put') {
        assert.deepEqual(JSON.parse(String(config.data)), {
          session_auto_logout: false,
        })
      }
      return response(config, 200, operation.reply)
    }
    assert.equal(isCurrentLoginSessionScope(scope), true)
    assert.deepEqual(await operation.run(), operation.reply)
    assert.equal(calls, 1)
  })
}

test('list passes the observer signal and accepts nullable session metadata', async () => {
  const controller = new AbortController()
  api.defaults.adapter = async (config) => {
    assert.equal(config.signal, controller.signal)
    assert.deepEqual(config.authScope, scope)
    return response(config, 200, listReply)
  }
  assert.deepEqual(await getLoginSessions(scope, controller.signal), listReply)
  controller.abort()
  let calls = 0
  api.defaults.adapter = async (config) => {
    calls++
    return response(config, 200, listReply)
  }
  await assert.rejects(getLoginSessions(scope, controller.signal), isCancel)
  assert.equal(calls, 0)
})

test('current-session revocation accepts the matching authoritative receipt', async () => {
  const reply = {
    success: true,
    data: { revoked_sid: scope.sessionId, current: true },
  }
  api.defaults.adapter = async (config) => response(config, 200, reply)
  assert.deepEqual(await revokeLoginSession(scope.sessionId, scope), reply)
})

test('business failures stay failures without global toasts', async () => {
  const failure = { success: false, message: 'fixture rejected' }
  api.defaults.adapter = async (config) => response(config, 200, failure)
  for (const operation of operations) {
    assert.deepEqual(await operation.run(), failure)
  }
})

const malformed: Array<[string, Operation['run'], unknown]> = [
  ['null envelope', operations[0].run, null],
  ['nonboolean success', operations[0].run, { ...listReply, success: 'true' }],
  [
    'missing list',
    operations[0].run,
    { success: true, session_auto_logout: true },
  ],
  ['nonarray list', operations[0].run, { ...listReply, data: {} }],
  ['missing policy', operations[0].run, { success: true, data: [] }],
  [
    'nonboolean policy',
    operations[0].run,
    { ...listReply, session_auto_logout: 1 },
  ],
  [
    'invalid row',
    operations[0].run,
    { ...listReply, data: [{ ...session, current: 1 }] },
  ],
  [
    'invalid timestamp',
    operations[0].run,
    { ...listReply, data: [{ ...session, created_at: '1' }] },
  ],
  ['missing revoke result', operations[1].run, { success: true }],
  [
    'wrong revoked target',
    operations[1].run,
    { success: true, data: { revoked_sid: scope.sessionId, current: false } },
  ],
  [
    'nonboolean current',
    operations[1].run,
    { success: true, data: { revoked_sid: 'other-session', current: 0 } },
  ],
  [
    'wrong current identity',
    operations[1].run,
    { success: true, data: { revoked_sid: 'other-session', current: true } },
  ],
  ...[-1, 0.5, '0'].map((count): [string, Operation['run'], unknown] => [
    `invalid revoked count ${JSON.stringify(count)}`,
    operations[2].run,
    { success: true, data: { revoked_count: count } },
  ]),
  ['missing revoked count', operations[2].run, { success: true, data: {} }],
  ['nonboolean settings success', operations[3].run, { success: 1 }],
]
for (const [name, run, payload] of malformed) {
  test(`rejects malformed success: ${name}`, async () => {
    api.defaults.adapter = async (config) => response(config, 200, payload)
    await assert.rejects(run())
  })
}

for (const replacement of [bundle('b', 99), bundle('b', 42, 'session-b')]) {
  test(`stale scope cannot dispatch as ${replacement.user.id}/${replacement.session.sid}`, async () => {
    applyAuthBundle(replacement, false)
    assert.equal(isCurrentLoginSessionScope(scope), false)
    let calls = 0
    api.defaults.adapter = async (config) => {
      calls++
      return response(config, 200, listReply)
    }
    for (const operation of operations) {
      await assert.rejects(operation.run(), isCancel)
    }
    assert.equal(calls, 0)
  })
}

for (const operation of writes) {
  test(`${operation.name} refreshes the same session before dispatching once`, async () => {
    applyAuthBundle({ ...bundle(), access_expires_at: now - 1 }, false)
    const order: string[] = []
    setDevelopmentAuthRefreshAdapter(async (config) => {
      order.push('refresh')
      assert.equal(held, 1)
      return response(config, 200, { success: true, data: bundle('prepared') })
    })
    api.defaults.adapter = async (config) => {
      order.push('write')
      assert.equal(held, 1)
      assert.equal(config.headers.get('Authorization'), 'Bearer prepared')
      assert.deepEqual(config.authScope, scope)
      return response(config, 200, operation.reply)
    }
    assert.deepEqual(await operation.run(), operation.reply)
    assert.deepEqual(order, ['refresh', 'write'])
    assert.equal(lockNames.length, 2)
  })

  test(`${operation.name} cannot dispatch after identity changes while queued`, async () => {
    const entered = deferred()
    const release = deferred()
    const blocker = withAuthCookieLock(async () => {
      entered.resolve()
      await release.promise
      applyAuthBundle(bundle('b', 99, 'session-b'), false)
    })
    await entered.promise
    let calls = 0
    api.defaults.adapter = async (config) => {
      calls++
      return response(config, 200, operation.reply)
    }
    const rejected = assert.rejects(operation.run(), isCancel)
    release.resolve()
    await Promise.all([blocker, rejected])
    assert.equal(calls, 0)
  })

  test(`${operation.name} cannot dispatch after identity changes during preflight refresh`, async () => {
    applyAuthBundle({ ...bundle(), access_expires_at: now - 1 }, false)
    let calls = 0
    setDevelopmentAuthRefreshAdapter(async (config) => {
      assert.equal(held, 1)
      assert.equal(config.headers.get('X-Auth-Session'), scope.sessionId)
      applyAuthBundle(bundle('b', 99, 'session-b'), false)
      return response(config, 200, { success: true, data: bundle('late-a') })
    })
    api.defaults.adapter = async (config) => {
      calls++
      return response(config, 200, operation.reply)
    }
    await assert.rejects(operation.run(), isCancel)
    assert.equal(calls, 0)
    assert.equal(useAuthStore.getState().auth.accessToken, 'b')
  })

  test(`${operation.name} releases its lock before 401 refresh and never replays`, async () => {
    let calls = 0
    let refreshes = 0
    api.defaults.adapter = async (config) => {
      calls++
      assert.equal(held, 1)
      throw new AxiosError(
        'private fixture detail',
        'ERR_BAD_REQUEST',
        config,
        undefined,
        response(config, 401, { code: 'AUTH_UNAUTHORIZED' })
      )
    }
    setDevelopmentAuthRefreshAdapter(async (config) => {
      refreshes++
      assert.equal(held, 1)
      return response(config, 200, { success: true, data: bundle('refreshed') })
    })
    await assert.rejects(
      operation.run(),
      (error: unknown) =>
        error instanceof AxiosError && error.response?.status === 401
    )
    assert.equal(calls, 1)
    assert.equal(refreshes, 1)
    assert.equal(lockNames.length, 2)
    assert.equal(useAuthStore.getState().auth.accessToken, 'refreshed')
  })
}

for (const operation of operations) {
  for (const status of [200, 503]) {
    test(`${operation.name}: late ${status} cannot affect the replacement identity or toast`, async () => {
      const entered = deferred()
      const release = deferred()
      api.defaults.adapter = async (config) => {
        entered.resolve()
        await release.promise
        if (status === 503) {
          throw new AxiosError(
            'private fixture detail',
            'ERR_BAD_RESPONSE',
            config,
            undefined,
            response(config, status, { message: 'private server detail' })
          )
        }
        return response(config, status, operation.reply)
      }
      const pending = operation.run()
      const rejected =
        status === 200
          ? assert.rejects(pending, isCancel)
          : assert.rejects(pending)
      await entered.promise
      applyAuthBundle(bundle('b', 99, 'session-b'), false)
      release.resolve()
      await rejected
      assert.equal(useAuthStore.getState().auth.user?.id, 99)
      assert.equal(useAuthStore.getState().auth.session?.sid, 'session-b')
      assert.equal(useAuthStore.getState().auth.accessToken, 'b')
      assert.deepEqual(toasts, [])
    })
  }
}
