/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import assert from 'node:assert/strict'
import { after, test } from 'node:test'

import { Window } from 'happy-dom'

const dom = new Window({ url: 'https://console.example.test/' })
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'Node',
  'Element',
  'Event',
  'MutationObserver',
  'getComputedStyle',
  'scrollTo',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: dom[key],
  })
}
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
after(() => dom.close())

const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const { createMemoryHistory, createRootRoute, createRouter, RouterProvider } =
  await import('@tanstack/react-router')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { useAuthStore } = await import('@/stores/auth-store')
const { ROLE } = await import('@/lib/roles')
const { api } = await import('@/lib/api')
const { RSSFeedsSection } = await import('./rss-feeds-section')
const { SettingsPageProvider } =
  await import('../components/settings-page-context')
const i18n = createInstance()
await i18n.use(initReactI18next).init({ lng: 'en', resources: {} })

async function waitFor(check: () => boolean) {
  for (let i = 0; i < 50 && !check(); i++) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 10))
    })
  }
  assert.ok(check(), document.body.textContent ?? '')
}

test('keeps the original feed draft and revision across a concurrent refresh', async () => {
  const originalAuth = useAuthStore.getState().auth
  const originalGet = api.get
  const originalPut = api.put
  const feed = {
    id: 'news',
    name: 'Original',
    url: 'https://example.test/feed',
    enabled: true,
  }
  const latest = { revision: 2, feeds: [{ ...feed, name: 'Other editor' }] }
  let stored = { revision: 1, feeds: [feed] }
  const writes: unknown[] = []
  api.get = (async (path) => {
    assert.equal(path, '/api/rss/settings')
    return { data: { success: true, data: stored } }
  }) as typeof api.get
  api.put = (async (path, input) => {
    assert.equal(path, '/api/rss/settings')
    writes.push(input)
    throw new Error('RSS revision conflict')
  }) as typeof api.put
  const role = ROLE.SUPER_ADMIN
  useAuthStore.setState({
    auth: {
      ...originalAuth,
      user: { id: 5, username: 'fixture', role },
      session: {
        sid: 'fixture',
        current: true,
        login_method: 'password',
        ip: null,
        user_agent: null,
        created_at: 0,
        last_active_at: 0,
        expires_at: 4000000000,
      },
    },
  })
  const container = document.createElement('div')
  const actions = document.createElement('div')
  document.body.append(container, actions)
  const root = createRoot(container)
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const router = createRouter({
    routeTree: createRootRoute({ component: RSSFeedsSection }),
    history: createMemoryHistory({ initialEntries: ['/'] }),
  })
  const button = (name: string) =>
    [...document.querySelectorAll('button')].find((b) => b.textContent === name)
  try {
    await act(async () => {
      await router.load()
      root.render(
        <QueryClientProvider client={client}>
          <I18nextProvider i18n={i18n}>
            <SettingsPageProvider actionsContainer={actions}>
              <RouterProvider router={router} />
            </SettingsPageProvider>
          </I18nextProvider>
        </QueryClientProvider>
      )
    })
    const selector = '[aria-label="Remove Original"]'
    await waitFor(() => Boolean(container.querySelector(selector)))
    await act(async () =>
      container.querySelector<HTMLButtonElement>(selector)!.click()
    )
    await waitFor(() => button('Save feeds')?.disabled === false)
    stored = latest
    await act(async () => {
      await client.refetchQueries({ queryKey: ['rss-settings'] })
    })
    await waitFor(
      () =>
        client.getQueryData<{ revision: number }>([
          'rss-settings',
          5,
          'fixture',
          role,
        ])?.revision === 2
    )
    assert.equal(
      container.querySelector('[aria-label="Remove Other editor"]'),
      null
    )
    assert.equal(button('Save feeds')?.disabled, false)
    await act(async () => button('Save feeds')!.click())
    await waitFor(() => Boolean(container.querySelector('[role=alert]')))
    assert.deepEqual(writes, [{ revision: 1, feeds: [] }])
    assert.equal(
      container.querySelector('[aria-label="Remove Other editor"]'),
      null
    )
    await act(async () => button('Discard changes and reload')!.click())
    await waitFor(() =>
      Boolean(container.querySelector('[aria-label="Remove Other editor"]'))
    )
    assert.equal(button('Save feeds')?.disabled, true)
    assert.equal(writes.length, 1)
  } finally {
    await act(async () => root.unmount())
    client.clear()
    container.remove()
    actions.remove()
    useAuthStore.setState({ auth: originalAuth })
    api.get = originalGet
    api.put = originalPut
  }
})
