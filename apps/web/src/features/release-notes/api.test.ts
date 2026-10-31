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
  canManageReleaseNotes,
  getLatestUnreadReleaseNote,
  isCurrentReleaseNoteScope,
  listReleaseNotes,
  markReleaseNoteRead,
  publishReleaseNote,
  releaseNoteInputSchema,
  type ReleaseNoteScope,
} from './api'
import type { ReleaseNote } from './types'

const now = 2_000_000_000
const scope: ReleaseNoteScope = {
  userId: 42,
  sessionId: 'release-session-a',
  role: 10,
}
const note: ReleaseNote = {
  id: 9,
  version: 'v1.2.3',
  revision: 2,
  content: '\n  - 修复登录问题。\n',
  published_at: now - 100,
  published_by: 7,
}
const input = { version: note.version, content: note.content }
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

function bundle(
  token = 'initial',
  userId = scope.userId,
  sessionId = scope.sessionId,
  role = scope.role
): AuthBundle {
  return {
    access_token: token,
    token_type: 'Bearer',
    access_expires_at: now + 600,
    user: { id: userId, username: 'synthetic', role },
    session: {
      sid: sessionId,
      current: true,
      login_method: 'password',
      ip: '',
      user_agent: '',
      created_at: now - 100,
      last_active_at: now,
      expires_at: now + 3600,
    },
  }
}

function response(
  config: Parameters<AxiosAdapter>[0],
  status: number,
  data: unknown
): AxiosResponse {
  return { config, status, statusText: 'fixture', headers: {}, data }
}

function assertRequest(config: Parameters<AxiosAdapter>[0], write = false) {
  assert.equal(config.authScope?.userId, scope.userId)
  assert.equal(config.authScope?.sessionId, scope.sessionId)
  assert.equal(config.skipBusinessError, true)
  assert.equal(config.skipErrorHandler, true)
  assert.equal(config.withCredentials, true)
  if (write) {
    assert.equal(config.skipAuthRefresh, true)
    assert.equal(config.timeout, AUTH_COOKIE_TIMEOUT_MS)
    assert.equal(held, 1)
    assert.equal(storage.get(cookieKey), undefined)
  }
}

Object.defineProperty(globalThis, 'isSecureContext', {
  configurable: true,
  value: true,
})
Object.defineProperty(globalThis, 'window', {
  configurable: true,
  value: {
    location: {
      origin: 'https://console.example.test',
      pathname: '/operations/releases',
    },
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
    throw new Error('Unexpected refresh in release-note fixture')
  })
  toast.error = toast.success = (...args: Parameters<typeof toast.error>) => {
    toasts.push(args[0])
    return 'release-note-test'
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
    if (descriptor) {
      Object.defineProperty(globalThis, key, descriptor)
    } else {
      Reflect.deleteProperty(globalThis, key)
    }
  }
})

const operations = [
  { name: 'latest', run: () => getLatestUnreadReleaseNote(scope), data: note },
  { name: 'history', run: () => listReleaseNotes(scope), data: [note] },
  { name: 'publish', run: () => publishReleaseNote(input, scope), data: note },
  {
    name: 'acknowledge',
    run: () => markReleaseNoteRead(note.id, scope),
    data: null,
  },
]
const writes = operations.slice(2)

test('management accepts exactly the established administrator roles', () => {
  for (const role of [10, 100]) {
    assert.equal(
      canManageReleaseNotes(bundle('a', 42, scope.sessionId, role).user),
      true
    )
  }
  for (const role of [0, 1, 5, 6, 9, 11, 50, 99, 101, Number.NaN]) {
    assert.equal(
      canManageReleaseNotes(bundle('a', 42, scope.sessionId, role).user),
      false
    )
  }
  assert.equal(canManageReleaseNotes(null), false)
  assert.equal(canManageReleaseNotes(undefined), false)
  assert.equal(isCurrentReleaseNoteScope(scope), true)
})

test('history uses a bounded authenticated GET with the observer signal', async () => {
  const controller = new AbortController()
  api.defaults.adapter = async (config) => {
    assertRequest(config)
    assert.equal(config.method, 'get')
    const url = new URL(config.url ?? '', 'https://console.example.test')
    assert.equal(url.pathname, '/api/release-notes/admin')
    assert.equal(
      String(config.params?.limit ?? url.searchParams.get('limit')),
      '50'
    )
    assert.equal(config.signal, controller.signal)
    assert.notEqual(config.skipAuthRefresh, true)
    assert.equal(config.headers.get('Authorization'), 'Bearer initial')
    assert.equal(held, 0)
    return response(config, 200, { success: true, data: [note] })
  }
  assert.deepEqual(await listReleaseNotes(scope, controller.signal), [note])
})

test('an ordinary user can read and acknowledge the exact returned publication', async () => {
  applyAuthBundle(bundle('reader', 42, scope.sessionId, 1), false)
  const reader = { ...scope, role: 1 }
  const controller = new AbortController()
  const paths: string[] = []
  api.defaults.adapter = async (config) => {
    paths.push(config.url ?? '')
    assertRequest(config, config.method === 'post')
    assert.equal(config.headers.get('Authorization'), 'Bearer reader')
    if (config.method === 'get') {
      assert.equal(config.signal, controller.signal)
      return response(config, 200, { success: true, data: note })
    }
    assert.equal(config.data, undefined)
    return response(config, 200, { success: true, data: null })
  }
  assert.deepEqual(
    await getLatestUnreadReleaseNote(reader, controller.signal),
    note
  )
  assert.equal(await markReleaseNoteRead(note.id, reader), undefined)
  assert.deepEqual(paths, [
    '/api/release-notes/latest',
    '/api/release-notes/9/read',
  ])
})

test('publishing preserves exact copy and accepts another publisher’s replay receipt', async () => {
  let calls = 0
  api.defaults.adapter = async (config) => {
    calls++
    assertRequest(config, true)
    assert.equal(config.method, 'post')
    assert.equal(config.url, '/api/release-notes/admin')
    assert.deepEqual(JSON.parse(String(config.data)), input)
    assert.notEqual(note.published_by, scope.userId)
    return response(config, 200, { success: true, data: note })
  }
  assert.deepEqual(await publishReleaseNote(input, scope), note)
  assert.equal(calls, 1)
  assert.equal(useAuthStore.getState().auth.accessToken, 'initial')
})

test('input limits count UTF-8 bytes and preserve intentional surrounding whitespace', async () => {
  const content = `${'界'.repeat(10922)}ab`
  assert.equal(new TextEncoder().encode(content).length, 32768)
  const valid = { version: 'v1+utf8', content }
  assert.deepEqual(releaseNoteInputSchema.parse(valid), valid)
  assert.deepEqual(releaseNoteInputSchema.parse(input), input)
  const emoji = { version: 'v-emoji', content: '🙂' }
  assert.deepEqual(releaseNoteInputSchema.parse(emoji), emoji)
  assert.equal(
    releaseNoteInputSchema.safeParse({ ...valid, content: `${content}c` })
      .success,
    false
  )
  api.defaults.adapter = async (config) => {
    assert.deepEqual(JSON.parse(String(config.data)), valid)
    return response(config, 200, {
      success: true,
      data: { ...note, ...valid },
    })
  }
  assert.deepEqual(await publishReleaseNote(valid, scope), {
    ...note,
    ...valid,
  })
})

test('invalid or authority-bearing publication input is rejected before dispatch', async () => {
  const invalid = [
    { ...input, version: 'v1/../../admin' },
    { ...input, version: 'v'.repeat(65) },
    { ...input, content: ' \n\t ' },
    { ...input, content: 'before\0after' },
    { ...input, content: '\uD800' },
    { ...input, content: '\uDC00' },
    { ...input, published_by: 999 },
  ]
  let calls = 0
  api.defaults.adapter = async (config) => {
    calls++
    return response(config, 200, { success: true, data: note })
  }
  for (const candidate of invalid) {
    assert.equal(releaseNoteInputSchema.safeParse(candidate).success, false)
    await assert.rejects(publishReleaseNote(candidate, scope))
  }
  assert.equal(calls, 0)
})

test('invalid acknowledgement IDs cannot become request paths', async () => {
  let calls = 0
  api.defaults.adapter = async (config) => {
    calls++
    return response(config, 200, { success: true, data: null })
  }
  for (const id of [0, -1, 1.5, Number.MAX_SAFE_INTEGER + 1]) {
    await assert.rejects(markReleaseNoteRead(id, scope))
  }
  assert.equal(calls, 0)
})

test('all operations require a true success envelope with an explicit data field', async () => {
  for (const operation of operations) {
    for (const payload of [
      null,
      {},
      { success: true },
      { success: 'true', data: operation.data },
      {
        success: false,
        data: operation.data,
        message: 'private fixture detail',
      },
    ]) {
      api.defaults.adapter = async (config) => response(config, 200, payload)
      await assert.rejects(operation.run(), operation.name)
    }
  }
  api.defaults.adapter = async (config) =>
    response(config, 200, { success: true, data: {} })
  await assert.rejects(markReleaseNoteRead(note.id, scope))
})

test('publication DTOs enforce identity, revision, timestamp and text bounds', async () => {
  for (const patch of [
    { id: 0 },
    { published_by: Number.MAX_SAFE_INTEGER + 1 },
    { revision: 10001 },
    { revision: 0.5 },
    { published_at: 0 },
    { published_at: 8640000000001 },
    { version: ' v1' },
    { content: '\0' },
  ]) {
    api.defaults.adapter = async (config) =>
      response(config, 200, {
        success: true,
        data: { ...note, ...patch },
      })
    await assert.rejects(getLatestUnreadReleaseNote(scope))
  }
  const boundary = {
    ...note,
    id: Number.MAX_SAFE_INTEGER,
    published_by: Number.MAX_SAFE_INTEGER,
    revision: 10000,
    published_at: 8640000000000,
    version: 'v'.repeat(64),
  }
  api.defaults.adapter = async (config) =>
    response(config, 200, { success: true, data: boundary })
  assert.deepEqual(await getLatestUnreadReleaseNote(scope), boundary)
})

test('empty states are explicit and history never accepts more than 50 rows', async () => {
  api.defaults.adapter = async (config) =>
    response(config, 200, { success: true, data: null })
  assert.equal(await getLatestUnreadReleaseNote(scope), null)
  await assert.rejects(listReleaseNotes(scope))
  const rows = Array.from({ length: 50 }, (_, i) => ({
    ...note,
    id: i + 1,
    version: `v1.0.${i}`,
  }))
  for (const data of [[], rows]) {
    api.defaults.adapter = async (config) =>
      response(config, 200, { success: true, data })
    assert.deepEqual(await listReleaseNotes(scope), data)
  }
  api.defaults.adapter = async (config) =>
    response(config, 200, {
      success: true,
      data: [...rows, { ...note, id: 51 }],
    })
  await assert.rejects(listReleaseNotes(scope))
})

test('publication success cannot substitute a different version or different copy', async () => {
  for (const patch of [
    { version: 'v9.9.9' },
    { content: note.content.trim() },
  ]) {
    api.defaults.adapter = async (config) =>
      response(config, 200, {
        success: true,
        data: { ...note, ...patch },
      })
    await assert.rejects(publishReleaseNote(input, scope))
  }
})

test('changed account, session or role invalidates the original scope before any request', async () => {
  let calls = 0
  api.defaults.adapter = async (config) => {
    calls++
    return response(config, 200, { success: true, data: note })
  }
  for (const replacement of [
    bundle('b', 99),
    bundle('b', 42, 'release-session-b'),
    bundle('demoted', 42, scope.sessionId, 1),
  ]) {
    applyAuthBundle(replacement, false)
    assert.equal(isCurrentReleaseNoteScope(scope), false)
    for (const operation of operations) {
      await assert.rejects(operation.run(), operation.name)
    }
  }
  assert.equal(calls, 0)
})

test('current ordinary users cannot dispatch publication management requests', async () => {
  applyAuthBundle(bundle('reader', 42, scope.sessionId, 1), false)
  const reader = { ...scope, role: 1 }
  let calls = 0
  api.defaults.adapter = async (config) => {
    calls++
    return response(config, 200, { success: true, data: note })
  }
  await assert.rejects(listReleaseNotes(reader))
  await assert.rejects(publishReleaseNote(input, reader))
  assert.equal(calls, 0)
})

test('queued writes recheck role after acquiring the Cookie lock', async () => {
  for (const operation of writes) {
    applyAuthBundle(bundle(), false)
    const entered = deferred()
    const release = deferred()
    const blocker = withAuthCookieLock(async () => {
      entered.resolve()
      await release.promise
      applyAuthBundle(bundle('demoted', 42, scope.sessionId, 1), false)
    })
    await entered.promise
    let calls = 0
    api.defaults.adapter = async (config) => {
      calls++
      return response(config, 200, {
        success: true,
        data: operation.data,
      })
    }
    const rejected = assert.rejects(operation.run(), operation.name)
    release.resolve()
    await Promise.all([blocker, rejected])
    assert.equal(calls, 0)
  }
})

test('preflight refresh cannot turn an old write into a changed-role operation', async () => {
  for (const operation of writes) {
    applyAuthBundle({ ...bundle(), access_expires_at: now - 1 }, false)
    let calls = 0
    let refreshes = 0
    setDevelopmentAuthRefreshAdapter(async (config) => {
      refreshes++
      return response(config, 200, {
        success: true,
        data: bundle('demoted', 42, scope.sessionId, 1),
      })
    })
    api.defaults.adapter = async (config) => {
      calls++
      return response(config, 200, {
        success: true,
        data: operation.data,
      })
    }
    await assert.rejects(operation.run(), operation.name)
    assert.equal(refreshes, 1)
    assert.equal(calls, 0)
    assert.equal(useAuthStore.getState().auth.user?.role, 1)
  }
})

test('late successful reads and writes are rejected after same-session demotion', async () => {
  for (const operation of operations) {
    applyAuthBundle(bundle(), false)
    const entered = deferred()
    const release = deferred()
    api.defaults.adapter = async (config) => {
      entered.resolve()
      await release.promise
      return response(config, 200, {
        success: true,
        data: operation.data,
      })
    }
    const rejected = assert.rejects(operation.run(), operation.name)
    await entered.promise
    applyAuthBundle(bundle('demoted', 42, scope.sessionId, 1), false)
    release.resolve()
    await rejected
    assert.equal(useAuthStore.getState().auth.accessToken, 'demoted')
    assert.deepEqual(toasts, [])
  }
})

test('late transport failures never toast or invalidate a replacement account', async () => {
  for (const operation of operations) {
    applyAuthBundle(bundle(), false)
    const entered = deferred()
    const release = deferred()
    api.defaults.adapter = async (config) => {
      entered.resolve()
      await release.promise
      throw new AxiosError(
        'private transport detail',
        'ERR_BAD_RESPONSE',
        config,
        undefined,
        response(config, 503, { message: 'private server detail' })
      )
    }
    const rejected = assert.rejects(operation.run(), operation.name)
    await entered.promise
    applyAuthBundle(bundle('b', 99, 'release-session-b'), false)
    release.resolve()
    await rejected
    assert.equal(useAuthStore.getState().auth.user?.id, 99)
    assert.equal(useAuthStore.getState().auth.accessToken, 'b')
    assert.deepEqual(toasts, [])
  }
})

test('an unknown write result neither refreshes nor replays the mutation', async () => {
  for (const operation of writes) {
    let calls = 0
    let refreshes = 0
    api.defaults.adapter = async (config) => {
      calls++
      assertRequest(config, true)
      throw new AxiosError('synthetic lost response', 'ERR_NETWORK', config)
    }
    setDevelopmentAuthRefreshAdapter(async (config) => {
      refreshes++
      return response(config, 200, {
        success: true,
        data: bundle('unexpected'),
      })
    })
    await assert.rejects(
      operation.run(),
      (error: unknown) =>
        error instanceof AxiosError && error.code === 'ERR_NETWORK'
    )
    assert.equal(calls, 1)
    assert.equal(refreshes, 0)
  }
})

test('a rejected write refreshes after releasing its lock but never resends', async () => {
  for (const operation of writes) {
    applyAuthBundle(bundle(), false)
    const locksBefore = lockNames.length
    let calls = 0
    let refreshes = 0
    api.defaults.adapter = async (config) => {
      calls++
      assertRequest(config, true)
      throw new AxiosError(
        'rejected',
        'ERR_BAD_REQUEST',
        config,
        undefined,
        response(config, 401, { code: 'AUTH_UNAUTHORIZED' })
      )
    }
    setDevelopmentAuthRefreshAdapter(async (config) => {
      refreshes++
      assert.equal(held, 1)
      assert.match(storage.get(cookieKey) ?? '', /^pending:/)
      return response(config, 200, {
        success: true,
        data: bundle('refreshed'),
      })
    })
    await assert.rejects(
      operation.run(),
      (error: unknown) =>
        error instanceof AxiosError && error.response?.status === 401
    )
    assert.equal(calls, 1)
    assert.equal(refreshes, 1)
    assert.equal(lockNames.length - locksBefore, 2)
    assert.equal(useAuthStore.getState().auth.accessToken, 'refreshed')
  }
})

test('a protected GET can refresh and recover within the same authorized session', async () => {
  let calls = 0
  let refreshes = 0
  api.defaults.adapter = async (config) => {
    calls++
    assertRequest(config)
    if (calls === 1) {
      throw new AxiosError(
        'expired access',
        'ERR_BAD_REQUEST',
        config,
        undefined,
        response(config, 401, { code: 'AUTH_UNAUTHORIZED' })
      )
    }
    assert.equal(config.headers.get('Authorization'), 'Bearer refreshed')
    return response(config, 200, { success: true, data: [note] })
  }
  setDevelopmentAuthRefreshAdapter(async (config) => {
    refreshes++
    return response(config, 200, {
      success: true,
      data: bundle('refreshed'),
    })
  })
  assert.deepEqual(await listReleaseNotes(scope), [note])
  assert.equal(calls, 2)
  assert.equal(refreshes, 1)
})

test('aborted observers cannot dispatch either GET', async () => {
  const controller = new AbortController()
  controller.abort()
  let calls = 0
  api.defaults.adapter = async (config) => {
    calls++
    return response(config, 200, { success: true, data: note })
  }
  await assert.rejects(
    getLatestUnreadReleaseNote(scope, controller.signal),
    isCancel
  )
  await assert.rejects(listReleaseNotes(scope, controller.signal), isCancel)
  assert.equal(calls, 0)
})
