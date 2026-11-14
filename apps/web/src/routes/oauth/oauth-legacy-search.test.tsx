/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
// @ts-expect-error Bun's test module is only available in the test runtime.
import { mock } from 'bun:test'
import assert from 'node:assert/strict'
import { after, test } from 'node:test'

import { Window } from 'happy-dom'

const dom = new Window({ url: 'https://web.example.test/oauth' })
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'Element',
  'Node',
  'Event',
  'MutationObserver',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: dom[key],
  })
}
Object.assign(globalThis, {
  IS_REACT_ACT_ENVIRONMENT: true,
  scrollTo: dom.scrollTo.bind(dom),
})
const errors: string[] = []
mock.module('sonner', () => ({
  toast: { error: (message: string) => errors.push(message) },
}))
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const {
  createRouter,
  createRootRoute,
  createRoute,
  createMemoryHistory,
  RouterProvider,
  Outlet,
} = await import('@tanstack/react-router')
const { Route } = await import('../(auth)/oauth')
const { saveFederatedCeremony } =
  await import('@/features/auth/lib/federated-oauth')
const i18next = (await import('i18next')).default
await i18next.init({ lng: 'en', resources: { en: { translation: {} } } })
after(() => dom.close())

async function visit(
  url: string,
  check: (router: ReturnType<typeof makeRouter>) => void
) {
  errors.length = 0
  const router = makeRouter(url)
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  try {
    await act(async () => {
      root.render(<RouterProvider router={router} />)
      await router.load()
    })
    for (
      let i = 0;
      i < 10 && router.state.location.pathname === '/oauth';
      i++
    ) {
      await act(async () => {
        await Promise.resolve()
      })
    }
    check(router)
  } finally {
    await act(() => root.unmount())
    router.history.destroy()
    host.remove()
  }
}
function makeRouter(url: string) {
  const root = createRootRoute({ component: Outlet })
  const auth = createRoute({ getParentRoute: () => root, id: '(auth)' })
  const legacy = createRoute({
    getParentRoute: () => auth,
    path: 'oauth',
    validateSearch: Route.options.validateSearch,
    component: Route.options.component,
  })
  const signIn = createRoute({
    getParentRoute: () => root,
    path: 'sign-in',
    component: () => <div>Sign in fixture</div>,
  })
  const callback = createRoute({
    getParentRoute: () => root,
    path: 'oauth/$provider',
    component: () => <div>Callback fixture</div>,
  })
  return createRouter({
    routeTree: root.addChildren([auth.addChildren([legacy]), signIn, callback]),
    history: createMemoryHistory({ initialEntries: [url] }),
    defaultPendingMinMs: 0,
  })
}

test('real router validates malformed legacy OAuth URL values before the component consumes search', async () => {
  for (const query of [
    'provider=1&code=ok&state=ok',
    'provider=%7B%22kind%22%3A%22wechat%22%7D&code=ok&state=ok',
    'provider=wechat&code=%5B%22injected%22%5D&state=ok',
    'provider=wechat&code=ok&state=%7B%22value%22%3A%22bad%22%7D',
    'provider=javascript%3Aalert(1)&code=ok&state=ok',
    'provider=wechat&code=ok&state=lmmfe_invalid&redirect=https%3A%2F%2Fevil.example',
  ]) {
    dom.sessionStorage.clear()
    await visit(`/oauth?${query}`, (router) => {
      assert.equal(router.state.location.pathname, '/sign-in', query)
      assert.deepEqual(errors, ['OAuth failed'])
    })
  }
})

test('exact-slug legacy WeChat ceremony preserves callback and safe redirect while dropping unsafe URL targets', async () => {
  const state = `lmmfe_${'a'.repeat(64)}`
  for (const redirect of [
    '/keys?view=all',
    'javascript:alert(1)',
    '//evil.example',
    'https://evil.example/keys',
  ]) {
    dom.sessionStorage.clear()
    seedLegacy(state)
    await visit(
      `/oauth?${new URLSearchParams({
        provider: 'wechat',
        code: 'synthetic-code',
        state,
        redirect,
      })}`,
      (router) => {
        assert.equal(router.state.location.pathname, '/oauth/wechat')
        const actual = new URLSearchParams(router.state.location.searchStr)
        assert.equal(actual.get('code'), 'synthetic-code')
        assert.equal(actual.get('state'), state)
        assert.equal(
          actual.get('redirect'),
          redirect.startsWith('/keys') ? redirect : null
        )
        assert.deepEqual(errors, [])
      }
    )
  }
})

function seedLegacy(state: string) {
  saveFederatedCeremony(
    dom.sessionStorage,
    {
      id: '71',
      revision: '1',
      kind: 'wechat',
      slug: 'wechat',
      name: 'Synthetic WeChat',
      icon: '',
      client_id: '',
      authorization_endpoint: '',
      scopes: '',
      qr_code_url: '',
      native_federated: true,
    },
    { state, expires_at: 2000000000, authorization_url: '' },
    'login'
  )
}

test('typed malicious code parameters are rejected even when the stored state and provider are valid', async () => {
  const state = `lmmfe_${'c'.repeat(64)}`
  for (const code of ['123', '["injected"]', '{"injected":"code"}']) {
    dom.sessionStorage.clear()
    seedLegacy(state)
    const query = new URLSearchParams({ provider: 'wechat', state, code })
    await visit(`/oauth?${query}`, (router) => {
      assert.equal(router.state.location.pathname, '/sign-in')
      assert.deepEqual(errors, ['OAuth failed'])
    })
  }
})
