/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
// @ts-expect-error Bun's test module is available only in the test runtime.
import { mock } from 'bun:test'
import assert from 'node:assert/strict'
import { after, afterEach, beforeEach, test } from 'node:test'

import { Window } from 'happy-dom'

import type { FederatedProvider } from '../lib/federated-oauth'

const dom = new Window({ url: 'https://web.example.test/sign-in' })
const old = new Map<string, PropertyDescriptor | undefined>()
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'Node',
  'Element',
  'Event',
  'isSecureContext',
  'IS_REACT_ACT_ENVIRONMENT',
]) {
  old.set(key, Object.getOwnPropertyDescriptor(globalThis, key))
}
let tail: Promise<unknown> = Promise.resolve(),
  lockCalls = 0
const locks = {
  request: (
    name: string,
    options: { signal?: AbortSignal },
    run: () => Promise<unknown>
  ) => {
    assert.equal(name, 'new-api:auth-refresh')
    lockCalls++
    const work = tail.then(() => {
      if (options.signal?.aborted) {
        throw new DOMException('cancelled', 'AbortError')
      }
      return run()
    })
    tail = work.catch(() => undefined)
    return work
  },
}
for (const key of [
  'window',
  'document',
  'HTMLElement',
  'Node',
  'Element',
  'Event',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: key === 'window' ? dom : dom[key],
  })
}
Object.defineProperty(globalThis, 'navigator', {
  configurable: true,
  value: { locks, userAgent: dom.navigator.userAgent },
})
Object.defineProperty(globalThis, 'isSecureContext', {
  configurable: true,
  value: true,
})
Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
  configurable: true,
  value: true,
})
const errors: string[] = [],
  navigations: string[] = []
mock.module('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}))
mock.module('sonner', () => ({
  toast: {
    error: (message: string) => errors.push(message),
    success: () => {},
    info: () => {},
  },
}))
Object.defineProperty(dom.location, 'assign', {
  configurable: true,
  value: (url: string) => navigations.push(url),
})
const { act, useLayoutEffect } = await import('react')
const { createRoot } = await import('react-dom/client')
const { api } = await import('@/lib/api')
const {
  capturePasswordLoginGuard,
  clearAuthentication,
  invalidatePendingLogins,
} = await import('@/lib/auth-session')
const { useBrowserAuthorization } = await import('./use-browser-authorization')
const originalAdapter = api.defaults.adapter,
  originalFetch = globalThis.fetch
const provider: FederatedProvider = {
  id: '71',
  kind: 'oidc',
  slug: 'synthetic-google',
  name: 'Google',
  icon: '',
  revision: '1',
  client_id: 'synthetic-client',
  authorization_endpoint: 'https://accounts.google.com/o/oauth2/v2/auth',
  scopes: 'openid profile email',
  qr_code_url: '',
  native_federated: true,
}
let hook!: ReturnType<typeof useBrowserAuthorization>,
  root: ReturnType<typeof createRoot> | undefined,
  posts = 0,
  fail = false
let originalURL = ''
function Harness() {
  const value = useBrowserAuthorization({} as never, true)
  useLayoutEffect(() => {
    hook = value
  })
  return null
}
const settle = () => new Promise<void>((r) => setImmediate(r))
function hold() {
  let release!: () => void
  tail = new Promise<void>((r) => {
    release = r
  })
  return release
}
beforeEach(async () => {
  clearAuthentication(false, 'idle')
  dom.localStorage.clear()
  dom.sessionStorage.clear()
  tail = Promise.resolve()
  lockCalls = posts = 0
  fail = false
  errors.length = navigations.length = 0
  globalThis.fetch = (() => {
    throw new Error('No HTTP in hook fixture')
  }) as typeof fetch
  api.defaults.adapter = async (config) => {
    posts++
    assert.equal(config.url, '/api/oauth/federated/71/begin')
    const state = `lmmfe_${posts.toString(16).padStart(64, '0')}`
    const url = new URL(provider.authorization_endpoint)
    for (const [key, value] of Object.entries({
      client_id: provider.client_id,
      redirect_uri: `https://web.example.test/oauth/${provider.slug}`,
      response_type: 'code',
      state,
      scope: 'openid profile email',
      nonce: 'synthetic-nonce',
      core_parameter: 'do-not-rebuild',
    })) {
      url.searchParams.set(key, value)
    }
    originalURL = url.toString()
    return {
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: fail
        ? { success: false }
        : {
            success: true,
            data: {
              state,
              authorization_url: originalURL,
              expires_at: 2000000000,
            },
          },
    }
  }
  const node = dom.document.createElement('div')
  dom.document.body.appendChild(node)
  const mounted = createRoot(node as unknown as HTMLElement)
  root = mounted
  await act(async () => mounted.render(<Harness />))
})
afterEach(async () => {
  const mounted = root
  if (mounted) await act(async () => mounted.unmount())
  root = undefined
  await tail
  api.defaults.adapter = originalAdapter
  globalThis.fetch = originalFetch
  clearAuthentication(false, 'idle')
  dom.document.body.innerHTML = ''
})
after(() => {
  dom.close()
  for (const [key, value] of old) {
    if (value) Object.defineProperty(globalThis, key, value)
    else Reflect.deleteProperty(globalThis, key)
  }
})
test('native Google-shaped catalog outer+inner share original click and preserve exact Core URL', async () => {
  const outer = capturePasswordLoginGuard()
  await act(async () => {
    await hook.handleFederatedLogin(provider)
  })
  assert.doesNotThrow(outer)
  assert.equal(dom.localStorage.getItem('new-api:auth-login-intent'), null)
  assert.match(
    dom.localStorage.getItem('new-api:auth-login-initialization') ?? '',
    /^[a-f0-9-]{36}$/
  )
  assert.equal(posts, 1)
  assert.equal(lockCalls, 2)
  assert.deepEqual(navigations, [originalURL])
  assert.equal(hook.isLoading, false)
  assert.deepEqual(errors, [])
})
test('cancel while native initialization queues releases busy and never starts begin', async () => {
  const release = hold()
  let pending!: Promise<void>
  await act(async () => {
    pending = hook.handleFederatedLogin(provider)
    await settle()
  })
  assert.equal(hook.isLoading, true)
  invalidatePendingLogins()
  release()
  await act(async () => {
    await pending
  })
  assert.equal(posts, 0)
  assert.deepEqual(navigations, [])
  assert.equal(hook.isLoading, false)
  assert.deepEqual(errors, [])
})
test('native begin failure releases busy; a fresh retry reuses I without changing C', async () => {
  fail = true
  await act(async () => {
    await hook.handleFederatedLogin(provider)
  })
  assert.equal(hook.isLoading, false)
  assert.equal(errors.length, 1)
  assert.equal(navigations.length, 0)
  const initialization = dom.localStorage.getItem(
    'new-api:auth-login-initialization'
  )
  fail = false
  await act(async () => {
    await hook.handleFederatedLogin(provider)
  })
  assert.equal(
    dom.localStorage.getItem('new-api:auth-login-initialization'),
    initialization
  )
  assert.equal(dom.localStorage.getItem('new-api:auth-login-intent'), null)
  assert.equal(hook.isLoading, false)
  assert.deepEqual(navigations, [originalURL])
})
test('unmount aborts queued preparation before any native request or navigation', async () => {
  const release = hold()
  let pending!: Promise<void>
  await act(async () => {
    pending = hook.handleFederatedLogin(provider)
    await settle()
    const mounted = root
    assert.ok(mounted)
    mounted.unmount()
  })
  root = undefined
  release()
  await pending
  assert.equal(posts, 0)
  assert.equal(navigations.length, 0)
})

test('real browser authorization Cookie guard leaves root Recovery as sole notification owner', async () => {
  const router = await import('@tanstack/react-router')
  mock.module('@tanstack/react-router', () => ({
    ...router,
    useNavigate: () => () => {},
  }))
  const { AuthCookieRecovery } =
    await import('@/components/auth-cookie-recovery')
  const mounted = root
  assert.ok(mounted)
  await act(async () =>
    mounted.render(
      <>
        <Harness />
        <AuthCookieRecovery />
      </>
    )
  )
  Object.defineProperty(navigator, 'locks', {
    configurable: true,
    value: undefined,
  })
  await act(async () => {
    await hook.handleFederatedLogin(provider)
  })
  assert.equal(posts, 0)
  assert.equal(document.querySelectorAll('[role="alert"]').length, 1)
  assert.match(document.body.textContent || '', /End this tab's local session/)
  assert.deepEqual(errors, [])
  assert.deepEqual(navigations, [])
})
