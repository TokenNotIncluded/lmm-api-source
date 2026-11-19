/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { Window } from 'happy-dom'

const dom = new Window({
  url: 'http://localhost/keys',
  settings: { device: { prefersReducedMotion: 'reduce' } },
})
dom.document.write('<!doctype html><html><head></head><body></body></html>')
for (const key of [
  'window',
  'document',
  'navigator',
  'history',
  'location',
  'HTMLElement',
  'HTMLButtonElement',
  'HTMLInputElement',
  'SVGElement',
  'customElements',
  'Node',
  'Element',
  'Event',
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
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: key === 'window' ? dom : dom[key],
  })
}
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
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
  Outlet,
} = await import('@tanstack/react-router')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { ApiKeysProvider } = await import('./api-keys-provider')
const { ApiKeysTable } = await import('./api-keys-table')
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
const recoveredKey = {
  id: 9,
  user_id: 1,
  name: 'Recovered key',
  key: 'sk-synthetic',
  status: 1,
  created_time: 1,
  accessed_time: 1,
  expired_time: -1,
  remain_quota: 1000,
  used_quota: 0,
  unlimited_quota: false,
  model_limits_enabled: false,
  model_limits: '',
  allow_ips: '',
  group: 'default',
}
const originalGet = api.get
let cleanup: (() => Promise<void>) | undefined
afterEach(async () => {
  await cleanup?.()
  cleanup = undefined
  api.get = originalGet
  document.body.replaceChildren()
})
after(() => dom.close())
const flush = () => new Promise((resolve) => setTimeout(resolve, 25))
async function mount(search = '') {
  const rootRoute = createRootRoute({ component: Outlet })
  const authRoute = createRoute({
    getParentRoute: () => rootRoute,
    id: '_authenticated',
    component: Outlet,
  })
  const keysRoute = createRoute({
    getParentRoute: () => authRoute,
    path: '/keys/',
    validateSearch: (value: Record<string, unknown>) => value,
    component: () => (
      <ApiKeysProvider>
        <ApiKeysTable creationMode='manual' />
      </ApiKeysProvider>
    ),
  })
  const router = createRouter({
    routeTree: rootRoute.addChildren([authRoute.addChildren([keysRoute])]),
    history: createMemoryHistory({ initialEntries: [`/keys${search}`] }),
  })
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  cleanup = async () => {
    await act(async () => root.unmount())
    client.clear()
    container.remove()
  }
  await act(async () => {
    root.render(
      <QueryClientProvider client={client}>
        <I18nextProvider i18n={i18n}>
          <RouterProvider router={router} />
        </I18nextProvider>
      </QueryClientProvider>
    )
    await router.load()
  })
  await act(async () => {
    await flush()
  })
  return { container, router }
}
function button(container: Element, name: string) {
  const result = [...container.querySelectorAll('button')].find(
    (node) => node.textContent?.trim() === name
  )
  assert.ok(result, `Missing button: ${name}`)
  return result
}
test('failed key reads remain an error and explicit retry recovers without inventing an empty account', async () => {
  let attempts = 0
  api.get = (async (url: string) => {
    if (url.startsWith('/api/token/')) {
      attempts++
      return {
        data:
          attempts === 1
            ? { success: false, message: 'Key service temporarily unavailable' }
            : { success: true, data: { items: [recoveredKey], total: 1 } },
      }
    }
    assert.equal(url, '/api/user/self/groups')
    return { data: { success: true, data: {} } }
  }) as typeof api.get
  const { container } = await mount()
  assert.match(container.textContent ?? '', /Failed to load API keys/)
  assert.match(
    container.textContent ?? '',
    /Key service temporarily unavailable/
  )
  assert.doesNotMatch(
    container.textContent ?? '',
    /Create your first API key|No API Keys Found/
  )
  assert.equal(attempts, 1)
  await act(async () => {
    button(container, 'Retry').click()
    await flush()
  })
  await act(async () => {
    await flush()
  })
  assert.equal(attempts, 2)
  assert.doesNotMatch(
    container.textContent ?? '',
    /Key service temporarily unavailable/
  )
  assert.match(container.textContent ?? '', /Recovered key/)
})
test('filtered empty lists provide clearing on both layouts and restore an unfiltered address', async () => {
  api.get = (async (url: string) => {
    if (url.startsWith('/api/token/')) {
      return { data: { success: true, data: { items: [], total: 0 } } }
    }
    assert.equal(url, '/api/user/self/groups')
    return { data: { success: true, data: {} } }
  }) as typeof api.get
  const { container, router } = await mount(
    '?filter=missing&status=%5B%222%22%5D'
  )
  assert.match(
    container.textContent ?? '',
    /No API keys match the current filters/
  )
  assert.doesNotMatch(container.textContent ?? '', /Create your first API key/)
  const clearButtons = [...container.querySelectorAll('button')].filter(
    (node) => node.textContent?.trim() === 'Clear filters'
  )
  const clearButton = clearButtons.at(-1)
  assert.ok(clearButton)
  await act(async () => {
    clearButton.click()
    await flush()
  })
  await act(async () => {
    await flush()
  })
  assert.equal(router.state.location.search.filter, undefined)
  assert.equal(router.state.location.search.status, undefined)
  assert.match(container.textContent ?? '', /Create your first API key/)
})
