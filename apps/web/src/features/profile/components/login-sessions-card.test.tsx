/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, beforeEach, test } from 'node:test'

import { AxiosError, type InternalAxiosRequestConfig } from 'axios'
import { Window } from 'happy-dom'

import type { LoginSession } from '@/stores/auth-store'

const dom = new Window({ url: 'https://console.example.test/profile' })
const descriptors = new Map<string, PropertyDescriptor | undefined>()
for (const key of [
  'window',
  'document',
  'navigator',
  'localStorage',
  'HTMLElement',
  'HTMLButtonElement',
  'HTMLInputElement',
  'SVGElement',
  'Node',
  'Element',
  'DocumentFragment',
  'Event',
  'KeyboardEvent',
  'MouseEvent',
  'PointerEvent',
  'FocusEvent',
  'CustomEvent',
  'MutationObserver',
  'ResizeObserver',
  'requestAnimationFrame',
  'cancelAnimationFrame',
  'getComputedStyle',
  'scrollTo',
] as const) {
  descriptors.set(key, Object.getOwnPropertyDescriptor(globalThis, key))
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: dom[key],
  })
}
descriptors.set(
  'IS_REACT_ACT_ENVIRONMENT',
  Object.getOwnPropertyDescriptor(globalThis, 'IS_REACT_ACT_ENVIRONMENT')
)
Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
  configurable: true,
  value: true,
})

const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const {
  createRootRoute,
  createRoute,
  createRouter,
  createMemoryHistory,
  RouterProvider,
} = await import('@tanstack/react-router')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { toast } = await import('sonner')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { LoginSessionsCard } = await import('./login-sessions-card')

const i18n = createInstance()
await i18n.use(initReactI18next).init({
  lng: 'en',
  resources: { en: { translation: {} } },
})
const originalAuth = useAuthStore.getState().auth
const originalAdapter = api.defaults.adapter
const originalSuccess = toast.success
const originalError = toast.error
const successes: string[] = []
const errors: string[] = []
const mounted: Array<{
  root: ReturnType<typeof createRoot>
  client: InstanceType<typeof QueryClient>
  container: HTMLDivElement
}> = []
const endpoint = '/api/user/sessions'
const key = (id = 7, sid = 'session-a') =>
  ['profile', 'login-sessions', id, sid] as const

function session(
  sid: string,
  current = false,
  ip = '198.51.100.10'
): LoginSession {
  return {
    sid,
    current,
    ip,
    login_method: 'password',
    user_agent: 'Mozilla/5.0 Chrome/100.0 Linux',
    created_at: 1_700_000_000,
    last_active_at: 1_700_000_100,
    expires_at: 4_000_000_000,
  }
}
function login(id = 7, sid = 'session-a') {
  useAuthStore.getState().auth.setBundle({
    access_token: `synthetic-${id}-${sid}`,
    token_type: 'Bearer',
    access_expires_at: 4_000_000_000,
    user: { id, username: `synthetic-${id}`, role: 1 },
    session: session(sid, true),
  })
}
function list(sid = 'session-a') {
  return {
    success: true,
    session_auto_logout: true,
    data: [
      session(sid, true, sid === 'session-a' ? '192.0.2.10' : '192.0.2.20'),
      session(`other-${sid}`),
    ],
  }
}
function response(
  config: InternalAxiosRequestConfig,
  data: unknown,
  status = 200
) {
  return { config, data, status, statusText: 'Synthetic', headers: {} }
}
function failure(config: InternalAxiosRequestConfig) {
  return new AxiosError(
    'Synthetic session failure',
    'ERR_BAD_RESPONSE',
    config,
    undefined,
    response(
      config,
      { success: false, message: 'Synthetic session failure' },
      503
    )
  )
}
function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => {
    resolve = done
  })
  return { promise, resolve }
}
async function flush() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 10))
  })
}
async function waitFor(predicate: () => boolean, message: string) {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    await flush()
    if (predicate()) return
  }
  assert.fail(message)
}
function button(
  label: string,
  within: ParentNode = document
): HTMLButtonElement {
  const found = [...within.querySelectorAll<HTMLButtonElement>('button')].find(
    (item) =>
      item.textContent?.trim() === label ||
      item.getAttribute('aria-label') === label
  )
  assert.ok(found, `button ${label}`)
  return found
}
async function click(label: string, within: ParentNode = document) {
  await act(async () => button(label, within).click())
  await flush()
}
function dialog() {
  const found = document.querySelector('[role="alertdialog"]')
  assert.ok(found, 'confirmation dialog')
  return found
}
async function confirm(label: string) {
  await click(label, dialog())
}
async function mount(
  handle: (config: InternalAxiosRequestConfig) => unknown = (config) =>
    list(config.authScope?.sessionId)
) {
  const requests: InternalAxiosRequestConfig[] = []
  api.defaults.adapter = async (config) => {
    assert.ok(
      config.url?.startsWith(endpoint),
      'only session endpoints are expected'
    )
    requests.push(config)
    return response(config, await handle(config))
  }
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  })
  const rootRoute = createRootRoute()
  const profile = createRoute({
    getParentRoute: () => rootRoute,
    path: '/profile',
    component: LoginSessionsCard,
  })
  const signIn = createRoute({
    getParentRoute: () => rootRoute,
    path: '/sign-in',
    component: () => <p>Signed out</p>,
  })
  const router = createRouter({
    routeTree: rootRoute.addChildren([profile, signIn]),
    history: createMemoryHistory({ initialEntries: ['/profile'] }),
  })
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  mounted.push({ root, client, container })
  await act(async () => {
    await router.load()
    root.render(
      <I18nextProvider i18n={i18n}>
        <QueryClientProvider client={client}>
          <RouterProvider router={router} />
        </QueryClientProvider>
      </I18nextProvider>
    )
  })
  await flush()
  return { client, router, container, requests }
}
type Fixture = Awaited<ReturnType<typeof mount>>
const reads = (fixture: Fixture) =>
  fixture.requests.filter((request) => request.method === 'get')
const writes = (fixture: Fixture) =>
  fixture.requests.filter((request) => request.method !== 'get')

beforeEach(() => {
  login()
  successes.length = 0
  errors.length = 0
  toast.success = (message) => {
    successes.push(String(message))
    return 'success'
  }
  toast.error = (message) => {
    errors.push(String(message))
    return 'error'
  }
})
afterEach(async () => {
  for (const fixture of mounted.splice(0)) {
    await act(async () => fixture.root.unmount())
    fixture.client.clear()
    fixture.container.remove()
  }
  api.defaults.adapter = originalAdapter
  toast.success = originalSuccess
  toast.error = originalError
  useAuthStore.setState({ auth: originalAuth })
})
after(() => {
  dom.close()
  for (const [name, descriptor] of descriptors) {
    if (descriptor) Object.defineProperty(globalThis, name, descriptor)
    else Reflect.deleteProperty(globalThis, name)
  }
})

test('lists scoped devices and refreshes after revoke, revoke-others and settings', async () => {
  let rows = [
    session('session-a', true, '192.0.2.10'),
    session('other-one'),
    session('other-two'),
  ]
  let enabled = true
  const fixture = await mount((config) => {
    assert.deepEqual(config.authScope, { userId: 7, sessionId: 'session-a' })
    if (config.method === 'get') {
      assert.ok(config.signal)
      return { success: true, data: rows, session_auto_logout: enabled }
    }
    if (config.method === 'delete') {
      assert.equal(config.url, `${endpoint}/other-one`)
      rows = rows.filter((row) => row.sid !== 'other-one')
      return {
        success: true,
        data: { revoked_sid: 'other-one', current: false },
      }
    }
    if (config.method === 'post') {
      assert.equal(config.url, `${endpoint}/revoke-others`)
      rows = rows.filter((row) => row.current)
      return { success: true, data: { revoked_count: 1 } }
    }
    assert.equal(config.method, 'put')
    assert.equal(config.url, `${endpoint}/settings`)
    assert.deepEqual(JSON.parse(String(config.data)), {
      session_auto_logout: false,
    })
    enabled = false
    return { success: true }
  })
  await click('Expand', fixture.container)
  await waitFor(
    () => fixture.container.textContent?.includes('192.0.2.10') === true,
    'devices load'
  )
  await click('Revoke', fixture.container)
  await confirm('Revoke')
  await waitFor(() => reads(fixture).length === 2, 'revoke refreshes the list')
  await click('Sign out other sessions', fixture.container)
  await confirm('Sign out others')
  await waitFor(
    () => reads(fixture).length === 3,
    'revoke-others refreshes the list'
  )
  const toggle = fixture.container.querySelector<HTMLElement>('[role="switch"]')
  assert.ok(toggle)
  await act(async () => toggle.click())
  await waitFor(
    () =>
      reads(fixture).length === 4 &&
      toggle.getAttribute('aria-checked') === 'false',
    'settings refresh uses the server value'
  )
  assert.deepEqual(successes, [
    'Session signed out',
    'Other sessions signed out',
    'Login session settings updated',
  ])
  assert.deepEqual(errors, [])
  assert.equal(writes(fixture).length, 3)
  assert.equal(fixture.router.state.location.pathname, '/profile')
})

for (const id of [8, 7]) {
  test(`a pending old list is cancelled and cannot replace user ${id}'s new session`, async () => {
    const pending = deferred<unknown>()
    const fixture = await mount((config) =>
      config.authScope?.sessionId === 'session-a'
        ? pending.promise
        : list('session-b')
    )
    await click('Expand', fixture.container)
    await waitFor(() => reads(fixture).length === 1, 'old list dispatches')
    const oldSignal = reads(fixture)[0]?.signal
    await act(async () => login(id, 'session-b'))
    await click('Expand', fixture.container)
    await waitFor(
      () => fixture.container.textContent?.includes('192.0.2.20') === true,
      'new devices load'
    )
    assert.equal(oldSignal?.aborted, true)
    await act(async () => pending.resolve(list('session-a')))
    await flush()
    assert.equal(fixture.container.textContent?.includes('192.0.2.10'), false)
    assert.ok(fixture.client.getQueryData(key(id, 'session-b')))
    assert.deepEqual(errors, [])
  })

  for (const others of [false, true]) {
    test(`old ${others ? 'revoke-others' : 'revoke'} confirmation cannot dispatch after user ${id} switches`, async () => {
      const fixture = await mount()
      await click('Expand', fixture.container)
      await waitFor(
        () => fixture.container.textContent?.includes('192.0.2.10') === true,
        'old devices load'
      )
      await click(
        others ? 'Sign out other sessions' : 'Revoke',
        fixture.container
      )
      const staleConfirm = button(
        others ? 'Sign out others' : 'Revoke',
        dialog()
      )
      await act(async () => {
        login(id, 'session-b')
        staleConfirm.click()
      })
      await flush()
      assert.equal(writes(fixture).length, 0)
      assert.equal(document.querySelector('[role="alertdialog"]'), null)
      await click('Expand', fixture.container)
      await waitFor(
        () => fixture.container.textContent?.includes('192.0.2.20') === true,
        'new devices load'
      )
      assert.equal(fixture.container.textContent?.includes('192.0.2.10'), false)
      assert.deepEqual(successes, [])
      assert.deepEqual(errors, [])
    })
  }

  for (const fail of [false, true]) {
    test(`late current-revoke ${fail ? 'error' : 'success'} cannot affect user ${id}'s replacement session`, async () => {
      const pending = deferred<void>()
      const fixture = await mount(async (config) => {
        if (config.method === 'get') return list(config.authScope?.sessionId)
        assert.equal(config.method, 'delete')
        assert.equal(config.url, `${endpoint}/session-a`)
        await pending.promise
        if (fail) throw failure(config)
        return {
          success: true,
          data: { revoked_sid: 'session-a', current: true },
        }
      })
      await click('Expand', fixture.container)
      await waitFor(
        () => fixture.container.textContent?.includes('192.0.2.10') === true,
        'old devices load'
      )
      await click('Sign out', fixture.container)
      await confirm('Sign out')
      assert.equal(writes(fixture).length, 1)
      await act(async () => login(id, 'session-b'))
      await click('Expand', fixture.container)
      await waitFor(
        () => fixture.container.textContent?.includes('192.0.2.20') === true,
        'replacement devices load'
      )
      const sentinel = { owner: id, sid: 'session-b' }
      fixture.client.setQueryData(['new-account-private'], sentinel)
      const newData = fixture.client.getQueryData(key(id, 'session-b'))
      const readCount = reads(fixture).length
      await act(async () => pending.resolve())
      await flush()
      assert.equal(useAuthStore.getState().auth.user?.id, id)
      assert.equal(useAuthStore.getState().auth.session?.sid, 'session-b')
      assert.equal(fixture.router.state.location.pathname, '/profile')
      assert.deepEqual(
        fixture.client.getQueryData(['new-account-private']),
        sentinel
      )
      assert.equal(fixture.client.getQueryData(key(id, 'session-b')), newData)
      assert.equal(
        fixture.client.getQueryState(key(id, 'session-b'))?.isInvalidated,
        false
      )
      assert.equal(reads(fixture).length, readCount)
      assert.deepEqual(successes, [])
      assert.deepEqual(errors, [])
    })
  }
}

for (const serverCurrent of [false, true]) {
  test(`server current=${serverCurrent} overrides the cached row's opposite current flag`, async () => {
    const target = serverCurrent ? 'session-a' : 'other-session'
    const fixture = await mount((config) => {
      if (config.method === 'get') {
        return {
          success: true,
          session_auto_logout: true,
          data: [session(target, !serverCurrent, '192.0.2.30')],
        }
      }
      assert.equal(config.method, 'delete')
      assert.equal(config.url, `${endpoint}/${target}`)
      return {
        success: true,
        data: { revoked_sid: target, current: serverCurrent },
      }
    })
    fixture.client.setQueryData(['private-before-revoke'], 'private')
    await click('Expand', fixture.container)
    await waitFor(
      () => fixture.container.textContent?.includes('192.0.2.30') === true,
      'cached row loads'
    )
    const label = serverCurrent ? 'Revoke' : 'Sign out'
    await click(label, fixture.container)
    await confirm(label)
    if (serverCurrent) {
      await waitFor(
        () => fixture.router.state.location.pathname === '/sign-in',
        'current revoke signs out'
      )
      assert.equal(useAuthStore.getState().auth.user, null)
      assert.equal(
        fixture.client.getQueryData(['private-before-revoke']),
        undefined
      )
    } else {
      await waitFor(() => reads(fixture).length === 2, 'other revoke refreshes')
      assert.equal(useAuthStore.getState().auth.session?.sid, 'session-a')
      assert.equal(fixture.router.state.location.pathname, '/profile')
      assert.equal(
        fixture.client.getQueryData(['private-before-revoke']),
        'private'
      )
      assert.deepEqual(successes, ['Session signed out'])
    }
    assert.deepEqual(errors, [])
  })
}

for (const malformed of [false, true]) {
  test(`${malformed ? 'missing list data' : 'list failure'} stays unavailable until Retry succeeds`, async () => {
    let count = 0
    const fixture = await mount((config) => {
      count += 1
      if (count > 1) return list()
      if (malformed) return { success: true }
      throw failure(config)
    })
    await click('Expand', fixture.container)
    await waitFor(
      () =>
        fixture.container.textContent?.includes(
          'Unable to load login sessions'
        ) === true,
      'failure is visible'
    )
    assert.equal(
      fixture.container.textContent?.includes('No active login sessions'),
      false
    )
    assert.equal(count, 1)
    await click('Retry', fixture.container)
    await waitFor(
      () => fixture.container.textContent?.includes('192.0.2.10') === true,
      'retry loads devices'
    )
    assert.equal(count, 2)
    assert.equal(
      fixture.container.textContent?.includes('Unable to load login sessions'),
      false
    )
  })
}
