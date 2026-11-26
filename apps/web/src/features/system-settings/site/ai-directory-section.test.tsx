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
const { DEFAULT_AI_DIRECTORY_LINKS } =
  await import('@/features/ai-directory/api')
const { AIDirectorySection } = await import('./ai-directory-section')
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

test('the standalone panel gates access, keeps a rejected draft and saves an explicit empty directory', async () => {
  const originalFetch = globalThis.fetch,
    originalPut = api.put
  const originalAuth = useAuthStore.getState().auth
  const link = {
    ...DEFAULT_AI_DIRECTORY_LINKS[0],
    name: 'Original',
    enabled: false,
  }
  let stored = [link],
    reads = 0,
    writes = 0
  globalThis.fetch = async (path) => {
    assert.equal(path, '/api/ai-directory')
    reads++
    return Response.json({ success: true, data: { links: stored } })
  }
  api.put = (async (path, body) => {
    assert.equal(path, '/api/ai-directory/links')
    assert.deepEqual(body, { links: [] })
    if (++writes === 1) throw new Error('forbidden')
    stored = []
    return { data: { success: true, data: { links: stored } } }
  }) as typeof api.put
  const session = {
    sid: 'fixture',
    current: true,
    login_method: 'password',
    ip: null,
    user_agent: null,
    created_at: 0,
    last_active_at: 0,
    expires_at: 4000000000,
  }
  const setRole = (role: number) =>
    useAuthStore.setState({
      auth: {
        ...originalAuth,
        user: { id: 5, username: 'fixture', role },
        session,
      },
    })
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  const cache = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const router = createRouter({
    routeTree: createRootRoute({ component: AIDirectorySection }),
    history: createMemoryHistory({ initialEntries: ['/'] }),
  })
  const button = (name: string) =>
    [...container.querySelectorAll<HTMLButtonElement>('button')].find(
      (b) => b.textContent === name
    )
  try {
    setRole(ROLE.ADMIN)
    await act(async () => {
      await router.load()
      root.render(
        <QueryClientProvider client={cache}>
          <I18nextProvider i18n={i18n}>
            <RouterProvider router={router} />
          </I18nextProvider>
        </QueryClientProvider>
      )
    })
    await waitFor(
      () => container.textContent?.includes('Access Forbidden') === true
    )
    assert.equal(reads, 0)
    await act(async () => setRole(ROLE.SUPER_ADMIN))
    await waitFor(() => container.textContent?.includes('Original') === true)
    assert.match(container.textContent ?? '', /Hidden from directory/)
    const remove = container.querySelector<HTMLButtonElement>(
      '[aria-label="Remove Original"]'
    )
    assert.ok(remove)
    await act(async () => remove.click())
    const save = button('Save websites')
    assert.ok(save)
    await act(async () => save.click())
    await waitFor(
      () =>
        container.querySelector('[role="alert"]')?.textContent ===
        'Failed to save'
    )
    assert.equal(cache.getQueryData(['ai-directory']), undefined)
    assert.equal(reads, 1)
    assert.equal(save.disabled, false)
    await act(async () => save.click())
    await waitFor(() => save.disabled && reads === 2)
    assert.deepEqual(cache.getQueryData(['ai-directory']), [])
    assert.equal(container.querySelector('[role="alert"]'), null)
    assert.equal(writes, 2)
  } finally {
    await act(async () => root.unmount())
    cache.clear()
    container.remove()
    useAuthStore.setState({ auth: originalAuth })
    globalThis.fetch = originalFetch
    api.put = originalPut
  }
})
