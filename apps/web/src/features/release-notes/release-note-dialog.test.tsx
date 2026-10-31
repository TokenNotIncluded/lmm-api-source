/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, test } from 'node:test'

import type {
  AxiosAdapter,
  AxiosResponse,
  InternalAxiosRequestConfig,
} from 'axios'
import { Window } from 'happy-dom'

import type { AuthBundle } from '@/stores/auth-store'

import type { ReleaseNoteScope } from './api'
import type { ReleaseNote } from './types'

const dom = new Window({
  url: 'https://console.example.test/operations/releases',
})
dom.document.write('<!doctype html><html><body></body></html>')

for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLButtonElement',
  'Element',
  'Node',
  'Event',
  'MouseEvent',
  'PointerEvent',
  'KeyboardEvent',
  'FocusEvent',
  'CustomEvent',
  'MutationObserver',
  'ResizeObserver',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: key === 'window' ? dom : dom[key],
  })
}
Object.defineProperties(globalThis, {
  IS_REACT_ACT_ENVIRONMENT: { configurable: true, value: true },
  isSecureContext: { configurable: true, value: true },
  getComputedStyle: {
    configurable: true,
    value: dom.getComputedStyle.bind(dom),
  },
  requestAnimationFrame: {
    configurable: true,
    value: (callback: FrameRequestCallback) =>
      setTimeout(() => callback(performance.now()), 0),
  },
  cancelAnimationFrame: {
    configurable: true,
    value: (handle: ReturnType<typeof setTimeout>) => clearTimeout(handle),
  },
})

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => {
    resolve = done
  })
  return { promise, resolve }
}

// Exercise the real auth-cookie queue without requiring a browser LockManager.
let lockTail: Promise<void> = Promise.resolve()
let lockRequests = 0
Object.defineProperty(dom.navigator, 'locks', {
  configurable: true,
  value: {
    async request<T>(
      _name: string,
      options: { signal?: AbortSignal },
      callback: () => Promise<T>
    ): Promise<T> {
      lockRequests++
      const predecessor = lockTail
      const release = deferred<void>()
      lockTail = release.promise
      await predecessor
      try {
        if (options.signal?.aborted) {
          throw new DOMException('Aborted', 'AbortError')
        }
        return await callback()
      } finally {
        release.resolve(undefined)
      }
    },
  },
})

const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const { AxiosError } = await import('axios')
const { toast } = await import('sonner')
const { api, applyAuthBundle, clearAuthentication } = await import('@/lib/api')
const { withAuthCookieLock, waitForAuthCookieIdle } =
  await import('@/lib/auth-cookie')
const { publishReleaseNote } = await import('./api')
const { ReleaseNoteDialog } = await import('./release-note-dialog')

const i18n = createInstance()
await i18n.use(initReactI18next).init({
  lng: 'en',
  resources: { en: { translation: {} } },
})

after(() => dom.close())

const alice: ReleaseNoteScope = { userId: 7, sessionId: 'session-a', role: 1 }
const bob: ReleaseNoteScope = { userId: 8, sessionId: 'session-b', role: 1 }
const latestURL = '/api/release-notes/latest'

function note(id = 17): ReleaseNote {
  return {
    id,
    version: `v1.0.${id}`,
    revision: 1,
    content: `Synthetic release ${id}`,
    published_at: 1_800_000_000,
    published_by: 1,
  }
}

function bundle(scope: ReleaseNoteScope): AuthBundle {
  const now = Math.floor(Date.now() / 1000)
  return {
    access_token: `release-${scope.userId}-${scope.sessionId}-${scope.role}`,
    token_type: 'Bearer',
    access_expires_at: now + 3600,
    user: {
      id: scope.userId,
      username: `synthetic-${scope.userId}`,
      role: scope.role,
    },
    session: {
      sid: scope.sessionId,
      current: true,
      login_method: 'password',
      ip: '',
      user_agent: '',
      created_at: now - 60,
      last_active_at: now,
      expires_at: now + 7200,
    },
  }
}

function queryKey(scope: ReleaseNoteScope) {
  return [
    'release-notes',
    'latest-unread',
    scope.userId,
    scope.sessionId,
    scope.role,
  ] as const
}

function response(
  config: InternalAxiosRequestConfig,
  data: unknown,
  status = 200
): AxiosResponse {
  return { config, data, status, statusText: 'Fixture', headers: {} }
}

function acknowledged(config: InternalAxiosRequestConfig): AxiosResponse {
  assert.equal(config.data, undefined, 'acknowledgement has no request body')
  return response(config, { success: true, data: null })
}

function unavailable(config: InternalAxiosRequestConfig) {
  return new AxiosError(
    'Synthetic unavailable transport',
    'ERR_BAD_RESPONSE',
    config,
    undefined,
    response(config, { success: false }, 503)
  )
}

function dialog() {
  return document.querySelector<HTMLElement>('[role="dialog"]')
}

function continueButton() {
  const button = [
    ...document.querySelectorAll<HTMLButtonElement>('button'),
  ].find((item) => item.textContent?.trim() === 'Got it, continue')
  assert.ok(button, 'acknowledgement button is visible')
  return button
}

async function turn() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 10))
  })
}

async function waitFor(predicate: () => boolean, message: string) {
  for (let attempt = 0; attempt < 50; attempt++) {
    await turn()
    if (predicate()) return
  }
  assert.fail(message)
}

async function expectVisible(value: ReleaseNote) {
  await waitFor(
    () =>
      Boolean(
        dialog()?.textContent?.includes(value.version) &&
        dialog()?.textContent?.includes(value.content)
      ),
    `release ${value.id} should be visible`
  )
}

async function acknowledge() {
  await act(async () => continueButton().click())
}

type Fixture = {
  client: InstanceType<typeof QueryClient>
  errors: string[]
  render: (visible?: boolean) => Promise<void>
  signIn: (scope: ReleaseNoteScope) => Promise<void>
}

async function withDialog(
  adapter: AxiosAdapter,
  run: (fixture: Fixture) => Promise<void>,
  scope: ReleaseNoteScope = alice
) {
  const originalAdapter = api.defaults.adapter
  const originalToastError = toast.error
  const errors: string[] = []
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: Number.POSITIVE_INFINITY },
      // The component must explicitly prevent acknowledgement replay.
      mutations: { retry: 1, retryDelay: 0 },
    },
  })
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  api.defaults.adapter = adapter
  toast.error = ((message) => {
    errors.push(String(message))
    return 'release-note-error'
  }) as typeof toast.error

  const render = async (visible = true) => {
    await act(async () => {
      root.render(
        <I18nextProvider i18n={i18n}>
          <QueryClientProvider client={client}>
            {visible ? <ReleaseNoteDialog /> : null}
          </QueryClientProvider>
        </I18nextProvider>
      )
    })
  }
  const signIn = async (next: ReleaseNoteScope) => {
    await act(async () => applyAuthBundle(bundle(next), false))
  }

  try {
    await signIn(scope)
    await render()
    await run({ client, errors, render, signIn })
  } finally {
    await act(async () => {
      root.unmount()
      clearAuthentication(false)
    })
    await waitForAuthCookieIdle()
    client.clear()
    container.remove()
    api.defaults.adapter = originalAdapter
    toast.error = originalToastError
    dom.localStorage.clear()
  }
}

test('published receipt is displayed to a user and only its exact ID is acknowledged once', async () => {
  const admin: ReleaseNoteScope = {
    userId: 1,
    sessionId: 'admin-session',
    role: 100,
  }
  const expected = note()
  const reads = new Set<string>()
  const requests: InternalAxiosRequestConfig[] = []
  let published: ReleaseNote | null = null

  await withDialog(
    async (config) => {
      requests.push(config)
      if (config.method === 'get' && config.url === latestURL) {
        const read = `${config.authScope?.userId}:${published?.id}`
        return response(config, {
          success: true,
          data: published && !reads.has(read) ? published : null,
        })
      }
      if (
        config.method === 'post' &&
        config.url === '/api/release-notes/admin'
      ) {
        assert.equal(config.authScope?.userId, admin.userId)
        assert.deepEqual(JSON.parse(String(config.data)), {
          version: expected.version,
          content: expected.content,
        })
        published = expected
        return response(config, { success: true, data: published })
      }
      if (
        config.method === 'post' &&
        config.url === `/api/release-notes/${expected.id}/read`
      ) {
        reads.add(`${config.authScope?.userId}:${expected.id}`)
        return acknowledged(config)
      }
      assert.fail(`Unexpected request: ${config.method} ${config.url}`)
    },
    async ({ client, render, signIn, errors }) => {
      assert.deepEqual(
        await publishReleaseNote(
          { version: expected.version, content: expected.content },
          admin
        ),
        expected
      )
      await signIn(alice)
      await expectVisible(expected)
      await acknowledge()
      await waitFor(
        () => client.getQueryData(queryKey(alice)) === null && !dialog(),
        'successful acknowledgement should close this note'
      )

      const writes = requests.filter((request) =>
        request.url?.endsWith('/read')
      )
      assert.equal(writes.length, 1)
      assert.equal(writes[0].authScope?.userId, alice.userId)
      assert.equal(writes[0].authScope?.sessionId, alice.sessionId)
      assert.equal(
        writes[0].headers.get('Authorization'),
        `Bearer ${bundle(alice).access_token}`
      )
      assert.deepEqual([...reads], [`${alice.userId}:${expected.id}`])

      await render(false)
      await render(true)
      await turn()
      assert.equal(dialog(), null)
      assert.equal(
        requests.filter(
          (request) =>
            request.url === latestURL &&
            request.authScope?.userId === alice.userId
        ).length,
        1,
        'acknowledgement must not fetch or cycle through older unread notes'
      )
      assert.deepEqual(errors, [])
    },
    admin
  )
})

test('acknowledging a release does not mark it read for another user', async () => {
  const expected = note()
  const readers = new Set<number>()
  await withDialog(
    async (config) => {
      const userId = config.authScope?.userId
      assert.ok(userId === alice.userId || userId === bob.userId)
      if (config.method === 'get' && config.url === latestURL) {
        return response(config, {
          success: true,
          data: readers.has(userId) ? null : expected,
        })
      }
      assert.equal(config.url, `/api/release-notes/${expected.id}/read`)
      assert.equal(config.method, 'post')
      readers.add(userId)
      return acknowledged(config)
    },
    async ({ client, signIn, errors }) => {
      await expectVisible(expected)
      await acknowledge()
      await waitFor(
        () => client.getQueryData(queryKey(alice)) === null,
        'first user should finish acknowledgement'
      )
      await signIn(bob)
      await expectVisible(expected)
      assert.deepEqual([...readers], [alice.userId])
      assert.equal(continueButton().disabled, false)
      await acknowledge()
      await waitFor(
        () => client.getQueryData(queryKey(bob)) === null,
        'second user should acknowledge independently'
      )
      assert.deepEqual([...readers], [alice.userId, bob.userId])
      assert.deepEqual(errors, [])
    }
  )
})

test('late acknowledgement cannot hide a newer note in the same query', async () => {
  const original = note()
  const newer = note(18)
  const release = deferred<void>()
  const writes: InternalAxiosRequestConfig[] = []
  await withDialog(
    async (config) => {
      if (config.method === 'get') {
        return response(config, { success: true, data: original })
      }
      writes.push(config)
      await release.promise
      return acknowledged(config)
    },
    async ({ client, errors }) => {
      try {
        await expectVisible(original)
        await acknowledge()
        await waitFor(() => writes.length === 1, 'acknowledgement dispatched')
        await act(async () => {
          client.setQueryData(queryKey(alice), newer)
        })
        await expectVisible(newer)
        release.resolve(undefined)
        await waitFor(
          () => client.isMutating() === 0,
          'old acknowledgement settled'
        )
        assert.equal(writes[0].url, `/api/release-notes/${original.id}/read`)
        assert.equal(writes.length, 1)
        assert.deepEqual(client.getQueryData(queryKey(alice)), newer)
        await expectVisible(newer)
        assert.equal(continueButton().disabled, false)
        assert.deepEqual(errors, [])
      } finally {
        release.resolve(undefined)
      }
    }
  )
})

test('account, SID and role changes isolate old acknowledgement callbacks', async () => {
  const replacements = [
    { scope: bob, receipt: 'success' },
    {
      scope: { ...alice, sessionId: 'replacement-session' },
      receipt: 'unavailable',
    },
    { scope: { ...alice, role: 10 }, receipt: 'malformed' },
  ] as const
  for (const replacement of replacements) {
    const original = note()
    const next = note(18)
    const release = deferred<void>()
    let gets = 0
    let writes = 0
    await withDialog(
      async (config) => {
        if (config.method === 'get') {
          gets++
          return response(config, {
            success: true,
            data: gets === 1 ? original : next,
          })
        }
        writes++
        await release.promise
        if (replacement.receipt === 'unavailable') {
          throw unavailable(config)
        }
        if (replacement.receipt === 'malformed') {
          return response(config, { success: true })
        }
        return acknowledged(config)
      },
      async ({ client, signIn, errors }) => {
        try {
          await expectVisible(original)
          await acknowledge()
          await waitFor(() => writes === 1, 'old acknowledgement dispatched')
          await signIn(replacement.scope)
          await expectVisible(next)
          assert.equal(continueButton().disabled, false)
          release.resolve(undefined)
          await waitFor(() => client.isMutating() === 0, 'old callback settled')
          assert.deepEqual(
            client.getQueryData(queryKey(replacement.scope)),
            next
          )
          assert.deepEqual(client.getQueryData(queryKey(alice)), original)
          await expectVisible(next)
          assert.deepEqual(errors, [])
          assert.equal(writes, 1)
        } finally {
          release.resolve(undefined)
        }
      }
    )
  }
})

test('an acknowledgement queued behind the auth lock cannot dispatch as the next user', async () => {
  const original = note()
  const next = note(18)
  let writes = 0
  await withDialog(
    async (config) => {
      if (config.method === 'get') {
        return response(config, {
          success: true,
          data: config.authScope?.userId === alice.userId ? original : next,
        })
      }
      writes++
      return acknowledged(config)
    },
    async ({ client, signIn, errors }) => {
      await expectVisible(original)
      const entered = deferred<void>()
      const release = deferred<void>()
      const blocker = withAuthCookieLock(async () => {
        entered.resolve(undefined)
        await release.promise
      })
      try {
        await entered.promise
        const before = lockRequests
        await acknowledge()
        await waitFor(
          () => lockRequests > before,
          'acknowledgement should be queued behind the held lock'
        )
        await signIn(bob)
        await expectVisible(next)
        release.resolve(undefined)
        await blocker
        await waitFor(() => client.isMutating() === 0, 'queued write settled')
        assert.equal(writes, 0)
        assert.deepEqual(client.getQueryData(queryKey(bob)), next)
        assert.equal(continueButton().disabled, false)
        assert.deepEqual(errors, [])
      } finally {
        release.resolve(undefined)
        await blocker
      }
    }
  )
})

test('malformed and unavailable acknowledgement receipts retain the note for explicit retry', async () => {
  for (const failure of ['malformed', 'unavailable'] as const) {
    const expected = note()
    const writes: InternalAxiosRequestConfig[] = []
    await withDialog(
      async (config) => {
        if (config.method === 'get') {
          return response(config, { success: true, data: expected })
        }
        writes.push(config)
        assert.equal(config.data, undefined)
        if (writes.length === 1) {
          if (failure === 'unavailable') {
            throw unavailable(config)
          }
          return response(config, { success: true })
        }
        return acknowledged(config)
      },
      async ({ client, errors }) => {
        await expectVisible(expected)
        await acknowledge()
        await waitFor(
          () => errors.length === 1 && client.isMutating() === 0,
          `${failure} acknowledgement should fail without automatic replay`
        )
        assert.deepEqual(errors, ['Failed to acknowledge release note'])
        assert.equal(writes.length, 1)
        assert.deepEqual(client.getQueryData(queryKey(alice)), expected)
        await expectVisible(expected)
        assert.equal(continueButton().disabled, false)
        await acknowledge()
        await waitFor(
          () => client.getQueryData(queryKey(alice)) === null && !dialog(),
          'explicit acknowledgement retry should succeed'
        )
        assert.deepEqual(
          writes.map((request) => request.url),
          [
            `/api/release-notes/${expected.id}/read`,
            `/api/release-notes/${expected.id}/read`,
          ]
        )
        assert.equal(errors.length, 1)
      }
    )
  }
})

test('malformed latest data remains a query error rather than a successful empty or read result', async () => {
  const requests: InternalAxiosRequestConfig[] = []
  await withDialog(
    async (config) => {
      requests.push(config)
      assert.equal(config.method, 'get')
      assert.equal(config.url, latestURL)
      return response(config, {
        success: true,
        data: { ...note(), revision: 'invalid' },
      })
    },
    async ({ client, errors }) => {
      await waitFor(
        () => client.getQueryState(queryKey(alice))?.status === 'error',
        'invalid latest DTO should fail its query'
      )
      assert.equal(client.getQueryData(queryKey(alice)), undefined)
      assert.equal(dialog(), null)
      assert.equal(requests.length, 1)
      assert.deepEqual(errors, [])
    }
  )
})

test('switching users aborts the old latest query and discards its late response', async () => {
  const old = note()
  const next = note(18)
  const release = deferred<void>()
  const gets: InternalAxiosRequestConfig[] = []
  let oldReturned = false
  await withDialog(
    async (config) => {
      assert.equal(config.method, 'get')
      assert.equal(config.url, latestURL)
      gets.push(config)
      if (config.authScope?.userId === alice.userId) {
        await release.promise
        oldReturned = true
        return response(config, { success: true, data: old })
      }
      return response(config, { success: true, data: next })
    },
    async ({ client, signIn, errors }) => {
      try {
        await waitFor(
          () => gets.length === 1,
          'first user latest request started'
        )
        assert.ok(gets[0].signal)
        await signIn(bob)
        await expectVisible(next)
        assert.equal(gets[0].signal.aborted, true)
        release.resolve(undefined)
        await waitFor(() => oldReturned, 'old transport response completed')
        assert.deepEqual(client.getQueryData(queryKey(bob)), next)
        assert.equal(client.getQueryData(queryKey(alice)), undefined)
        await expectVisible(next)
        assert.equal(gets.length, 2)
        assert.deepEqual(errors, [])
      } finally {
        release.resolve(undefined)
      }
    }
  )
})
