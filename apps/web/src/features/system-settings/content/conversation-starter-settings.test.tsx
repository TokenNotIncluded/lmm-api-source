/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
// @ts-expect-error Bun's module mock is available only in the test runtime.
import { mock as moduleMock } from 'bun:test'
import assert from 'node:assert/strict'
import { after, afterEach, beforeEach, test } from 'node:test'

import { AxiosError, type AxiosAdapter } from 'axios'
import { Window } from 'happy-dom'
import type { ReactNode } from 'react'

import type { AuthBundle } from '@/stores/auth-store'

const dom = new Window({
  url: 'https://console.example.test/system-settings/content/assistant',
})
dom.document.write('<!doctype html><html><head></head><body></body></html>')
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLInputElement',
  'HTMLTextAreaElement',
  'HTMLButtonElement',
  'HTMLDivElement',
  'SVGElement',
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
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: dom[key],
  })
}
Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
  configurable: true,
  value: true,
})
Object.defineProperty(globalThis, 'scrollTo', {
  configurable: true,
  value: dom.scrollTo.bind(dom),
})
// Layout animations have no bearing on routing, authority or CAS behavior;
// happy-dom's canceled native-animation promise otherwise rejects on unmount.
const transitions = await import('@/components/page-transition')
moduleMock.module('@/components/page-transition', () => ({
  ...transitions,
  FadeIn: ({ children }: { children?: ReactNode }) => <div>{children}</div>,
}))

const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider, notifyManager } =
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
const { Route: AuthenticatedRoute } =
  await import('@/routes/_authenticated/route')
const { Route: SettingsRoute } =
  await import('@/routes/_authenticated/system-settings/route')
const { Route: ContentRoute } =
  await import('@/routes/_authenticated/system-settings/content/$section')
const { useSidebarView } = await import('@/hooks/use-sidebar-view')
const { buildSettingsSearchIndex } =
  await import('../utils/settings-search-index')
const { getSystemSettingsNavGroups, SYSTEM_SETTINGS_VIEW } =
  await import('@/components/layout/config/system-settings.config')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { validPresetConfiguration } =
  await import('./assistant-preset-settings-api')
notifyManager.setScheduler(queueMicrotask)
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })

const initial =
  '[{"id":"custom","label":"Published label","prompt":"Published prompt"}]'
const originalAdapter = api.defaults.adapter
const previousAuth = useAuthStore.getState().auth
let remote = { configuration: initial, revision: 1, updated_at: 10 }
let failRead = false
let failWrite = false
const requests: {
  method: string
  url: string
  data?: { configuration: string; revision: number }
}[] = []

function bundle(role = 100, userId = 101): AuthBundle {
  return {
    access_token: `lmms_${'a'.repeat(43)}`,
    token_type: 'Bearer',
    access_expires_at: 2000000000,
    user: {
      id: userId,
      username: 'synthetic-administrator',
      role,
      developer_access_granted: true,
    },
    session: {
      sid: `session-${userId}`,
      current: true,
      login_method: 'password',
      ip: null,
      user_agent: null,
      created_at: 1,
      last_active_at: 1,
      expires_at: 2000000000,
    },
  }
}

beforeEach(() => {
  remote = { configuration: initial, revision: 1, updated_at: 10 }
  failRead = false
  failWrite = false
  requests.length = 0
  useAuthStore.getState().auth.setBundle(bundle())
  const adapter: AxiosAdapter = async (config) => {
    const data = config.data ? JSON.parse(String(config.data)) : undefined
    requests.push({ method: config.method ?? '', url: config.url ?? '', data })
    if (config.url === '/api/status') {
      return {
        config,
        status: 200,
        statusText: 'OK',
        headers: {},
        data: { success: true, data: {} },
      }
    }
    assert.equal(
      config.url,
      '/api/assistant/settings/pre-conversation-presets',
      'the actual route must not depend on legacy option capabilities'
    )
    assert.equal(config.skipAuthRefresh, true)
    assert.equal(
      config.authScope?.userId,
      useAuthStore.getState().auth.user?.id
    )
    const response = (status: number, data: unknown) => ({
      config,
      status,
      data,
      headers: {},
      statusText: String(status),
    })
    if (config.method === 'get') {
      if (failRead) {
        throw new AxiosError(
          'unavailable',
          undefined,
          config,
          undefined,
          response(503, { success: false })
        )
      }
      return response(200, { success: true, data: remote })
    }
    assert.equal(config.method, 'put')
    assert.deepEqual(Object.keys(data).sort(), ['configuration', 'revision'])
    if (failWrite) {
      throw new AxiosError(
        'unavailable',
        undefined,
        config,
        undefined,
        response(503, { success: false })
      )
    }
    if (data.revision !== remote.revision) {
      throw new AxiosError(
        'conflict',
        undefined,
        config,
        undefined,
        response(409, { success: false, code: 'ASSISTANT_HISTORY_CONFLICT' })
      )
    }
    remote = {
      configuration: data.configuration,
      revision: remote.revision + 1,
      updated_at: remote.updated_at + 1,
    }
    return response(200, { success: true, data: remote })
  }
  api.defaults.adapter = adapter
})

afterEach(() => {
  api.defaults.adapter = originalAdapter
  useAuthStore.setState({ auth: previousAuth })
})
after(() => dom.close())

function SidebarProbe() {
  const sidebar = useSidebarView()
  return (
    <output data-testid='sidebar-routes'>
      {JSON.stringify(
        sidebar.navGroups.flatMap((group) =>
          group.items.flatMap((item) => (item.url ? [item.url] : []))
        )
      )}
    </output>
  )
}

async function renderRoute(
  path = '/system-settings/content/assistant',
  menu = false
) {
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  })
  queryClient.setQueryData(['system-options'], {
    success: true,
    data: [],
    capabilities: { partial: true, read_only: true },
  })
  queryClient.setQueryData(['todos', 'all'], { total_unread_count: 0 })
  const rootRoute = createRootRoute({
    component: () => (
      <>
        {menu && <SidebarProbe />}
        <Outlet />
      </>
    ),
  })
  const authenticated = createRoute({
    getParentRoute: () => rootRoute,
    id: '_authenticated',
    component: Outlet,
    beforeLoad: (context) =>
      AuthenticatedRoute.options.beforeLoad?.(context as never),
  })
  const settings = createRoute({
    getParentRoute: () => authenticated,
    path: '/system-settings',
    component: SettingsRoute.options.component,
    beforeLoad: (context) =>
      SettingsRoute.options.beforeLoad?.(context as never),
  })
  const content = createRoute({
    getParentRoute: () => settings,
    path: '/content/$section',
    component: ContentRoute.options.component,
    beforeLoad: (context) =>
      ContentRoute.options.beforeLoad?.(context as never),
  })
  const router = createRouter({
    routeTree: rootRoute.addChildren([
      authenticated.addChildren([settings.addChildren([content])]),
      createRoute({
        getParentRoute: () => rootRoute,
        path: '/403',
        component: () => <p>Access Forbidden</p>,
      }),
      createRoute({
        getParentRoute: () => rootRoute,
        path: '/sign-in',
        component: () => <p>Session required</p>,
      }),
      createRoute({
        getParentRoute: () => rootRoute,
        path: '/dashboard/overview',
        component: () => <p>Dashboard</p>,
      }),
    ]),
    history: createMemoryHistory({
      initialEntries: [path],
    }),
  })
  await router.load()
  await act(async () => {
    root.render(
      <QueryClientProvider client={queryClient}>
        <I18nextProvider i18n={i18n}>
          <RouterProvider router={router} />
        </I18nextProvider>
      </QueryClientProvider>
    )
  })
  return {
    container,
    queryClient,
    router,
    cleanup: async () => {
      await act(async () => root.unmount())
      container.remove()
      queryClient.clear()
    },
  }
}

async function until(check: () => boolean, label: string) {
  for (let n = 0; n < 100; n++) {
    if (check()) return
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 5))
    })
  }
  assert.ok(check(), label)
}
function field(container: HTMLElement): HTMLInputElement {
  const input = container.querySelector<HTMLInputElement>(
    'input[aria-label="Button label"]'
  )
  assert.ok(input)
  return input
}
function button(container: HTMLElement, label: string): HTMLButtonElement {
  const panel =
    container.querySelector('[data-testid="assistant-preset-settings"]') ??
    container
  const button = Array.from(panel.querySelectorAll('button')).find(
    (button) => button.textContent?.trim() === label
  )
  assert.ok(button, label)
  return button
}
async function edit(container: HTMLElement, value: string) {
  await act(async () => {
    Object.getOwnPropertyDescriptor(
      dom.HTMLInputElement.prototype,
      'value'
    )?.set?.call(field(container), value)
    field(container).dispatchEvent(new Event('input', { bubbles: true }))
  })
}
async function click(container: HTMLElement, label: string) {
  await act(async () => {
    button(container, label).click()
  })
}

test('real assistant settings route saves presets independently of partial legacy options', async () => {
  const page = await renderRoute()
  try {
    await until(
      () => !!page.container.querySelector('input[aria-label="Button label"]'),
      'published editor'
    )
    assert.equal(field(page.container).value, 'Published label')
    assert.equal(requests.length, 1)
    await edit(page.container, 'Saved custom label')
    await click(page.container, 'Save Settings')
    await until(() => remote.revision === 2, 'acknowledged settings write')
    assert.equal(
      JSON.parse(remote.configuration)[0].label.default,
      'Saved custom label'
    )
    assert.equal(button(page.container, 'Save Settings').disabled, true)
    assert.deepEqual(
      requests.map((request) => request.method),
      ['get', 'put']
    )
  } finally {
    await page.cleanup()
  }
})

test('conflicts keep the complete draft and require an explicit reload and save', async () => {
  const page = await renderRoute()
  try {
    await until(
      () => !!page.container.querySelector('input[aria-label="Button label"]'),
      'published editor'
    )
    await edit(page.container, 'My unsaved label')
    remote = {
      configuration:
        '[{"id":"custom","label":"Other administrator","prompt":"New server prompt"}]',
      revision: 2,
      updated_at: 11,
    }
    await click(page.container, 'Save Settings')
    await until(
      () => page.container.textContent?.includes('Settings changed.') === true,
      'CAS conflict'
    )
    assert.equal(field(page.container).value, 'My unsaved label')
    assert.equal(button(page.container, 'Save Settings').disabled, true)
    await click(page.container, 'Keep draft and reload revision')
    await until(
      () => page.container.textContent?.includes('Current version') === true,
      'review fresh revision'
    )
    assert.equal(field(page.container).value, 'My unsaved label')
    assert.ok(page.container.textContent?.includes('New server prompt'))
    assert.equal(
      requests.filter((request) => request.method === 'put').length,
      1
    )
    await click(page.container, 'Save Settings')
    await until(() => remote.revision === 3, 'explicit retry')
    assert.equal(requests.at(-1)?.data?.revision, 2)
    assert.equal(
      JSON.parse(remote.configuration)[0].label.default,
      'My unsaved label'
    )
  } finally {
    await page.cleanup()
  }
})

test('failed preset saves retain the draft and only acknowledged writes invalidate published starters', async () => {
  const page = await renderRoute()
  const key = ['assistant-pre-conversation-presets', 'access-v3', 'en']
  page.queryClient.setQueryData(key, { presets: [] })
  try {
    await until(
      () => !!page.container.querySelector('input[aria-label="Button label"]'),
      'published editor'
    )
    await edit(page.container, 'Retained custom label')
    failWrite = true
    await click(page.container, 'Save Settings')
    await until(
      () => page.container.textContent?.includes('Save failed') === true,
      'rejected save'
    )
    assert.equal(field(page.container).value, 'Retained custom label')
    assert.equal(remote.revision, 1)
    assert.equal(page.queryClient.getQueryState(key)?.isInvalidated, false)
    failWrite = false
    await click(page.container, 'Save Settings')
    await until(
      () => page.queryClient.getQueryState(key)?.isInvalidated === true,
      'published cache invalidation'
    )
    assert.equal(remote.revision, 2)
    assert.equal(
      JSON.parse(remote.configuration)[0].label.default,
      'Retained custom label'
    )
  } finally {
    await page.cleanup()
  }
})

test('L5 can read the published editor and cannot submit a change', async () => {
  useAuthStore.getState().auth.setBundle(bundle(10))
  const page = await renderRoute()
  try {
    await until(
      () => !!page.container.querySelector('input[aria-label="Button label"]'),
      'L5 read'
    )
    assert.equal(field(page.container).value, 'Published label')
    assert.equal(field(page.container).disabled, true)
    assert.equal(button(page.container, 'Save Settings').disabled, true)
    assert.ok(page.container.textContent?.includes('Read only'))
    await click(page.container, 'Save Settings')
    assert.deepEqual(
      requests.map((request) => request.method),
      ['get']
    )
  } finally {
    await page.cleanup()
  }
})

test('actual parent guard rejects other L5 settings and revoked sessions', async () => {
  useAuthStore.getState().auth.setBundle(bundle(10))
  let page = await renderRoute('/system-settings/content/announcements')
  try {
    await until(
      () => page.container.textContent?.includes('Access Forbidden') === true,
      'L5 other settings denied'
    )
    assert.equal(requests.length, 0)
    assert.equal(page.router.state.location.pathname, '/403')
  } finally {
    await page.cleanup()
  }
  page = await renderRoute('/system-settings/content/assistant/')
  try {
    await until(
      () => !!page.container.querySelector('input[aria-label="Button label"]'),
      'L5 canonical assistant route'
    )
    await act(async () => {
      const auth = useAuthStore.getState().auth
      useAuthStore.setState({
        auth: {
          ...auth,
          user: null,
          session: null,
          accessToken: '',
          bootstrapState: 'complete',
        },
      })
      await page.router.invalidate()
    })
    await until(
      () => page.container.textContent?.includes('Session required') === true,
      'revoked session redirected'
    )
    assert.equal(
      page.container.querySelector('input[aria-label="Button label"]'),
      null
    )
    assert.equal(
      requests.filter((request) => request.method === 'put').length,
      0
    )
  } finally {
    await page.cleanup()
  }
})

test('L5 discovers only the assistant settings destination while L6 retains the full directory', async () => {
  useAuthStore.getState().auth.setBundle(bundle(10))
  const page = await renderRoute('/dashboard/overview', true)
  try {
    const target = '/system-settings/content/assistant'
    await until(
      () =>
        page.container
          .querySelector('[data-testid="sidebar-routes"]')
          ?.textContent?.includes(target) === true,
      'L5 sidebar starter link'
    )
    const routeText = page.container.querySelector(
      '[data-testid="sidebar-routes"]'
    )?.textContent
    assert.ok(routeText)
    const rootRoutes = JSON.parse(routeText) as string[]
    assert.deepEqual(
      rootRoutes.filter((route) => route.startsWith('/system-settings')),
      [target]
    )
    const t = i18n.getFixedT('en')
    assert.deepEqual(
      buildSettingsSearchIndex(t, undefined, 10).flatMap((group) =>
        group.entries.map((entry) => entry.url)
      ),
      [target]
    )
    assert.deepEqual(
      SYSTEM_SETTINGS_VIEW.getNavGroups(t).flatMap((group) =>
        group.items.flatMap(
          (section) => section.items?.map((entry) => entry.url) ?? []
        )
      ),
      [target]
    )
    const full = buildSettingsSearchIndex(t, undefined, 100).flatMap(
      (group) => group.entries
    )
    assert.ok(full.length > 20)
    assert.equal(
      full.length,
      buildSettingsSearchIndex(t, getSystemSettingsNavGroups(t, 100)).flatMap(
        (group) => group.entries
      ).length
    )
    assert.equal(buildSettingsSearchIndex(t, undefined, 1).length, 0)
  } finally {
    await page.cleanup()
  }
})

test('failed reads do not invent default editor content and low levels do not fetch settings', async () => {
  failRead = true
  let page = await renderRoute()
  try {
    await until(
      () =>
        page.container.textContent?.includes('Unable to load settings') ===
        true,
      'failed settings read'
    )
    assert.equal(
      page.container.querySelector('input[aria-label="Button label"]'),
      null
    )
  } finally {
    await page.cleanup()
  }
  requests.length = 0
  useAuthStore.getState().auth.setBundle(bundle(1))
  page = await renderRoute()
  try {
    await until(
      () => page.container.textContent?.includes('Access Forbidden') === true,
      'level denied'
    )
    assert.equal(requests.length, 0)
  } finally {
    await page.cleanup()
  }
})

test('preset validation counts Unicode characters and rejects empty or privilege-bearing entries', () => {
  assert.equal(validPresetConfiguration(''), true)
  assert.equal(validPresetConfiguration('[]'), true)
  assert.equal(
    validPresetConfiguration(
      JSON.stringify([
        { id: 'custom', label: '中'.repeat(80), prompt: '😀'.repeat(2000) },
      ])
    ),
    true
  )
  for (const raw of [
    'null',
    '{}',
    '[null]',
    '[{"id":"custom","label":"Label","prompt":""}]',
    '[{"id":"custom","label":{"en":"Label"},"prompt":"Prompt"}]',
    '[{"id":"custom","label":"Label","prompt":"Prompt","tools":[]}]',
    JSON.stringify([
      { id: 'custom', label: 'Label', prompt: '😀'.repeat(2001) },
    ]),
  ]) {
    assert.equal(validPresetConfiguration(raw), false, raw.slice(0, 100))
  }
})
