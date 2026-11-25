/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { Window } from 'happy-dom'

const dom = new Window({ url: 'https://directory.example' })
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'Node',
  'Element',
  'Event',
  'MutationObserver',
  'self',
  'location',
  'history',
  'HTMLInputElement',
  'SVGElement',
  'MouseEvent',
  'CustomEvent',
  'ResizeObserver',
  'scrollTo',
  'getComputedStyle',
  'requestAnimationFrame',
  'cancelAnimationFrame',
  'sessionStorage',
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

const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { ROLE } = await import('@/lib/roles')

const { createMemoryHistory, createRootRoute, createRouter, RouterProvider } =
  await import('@tanstack/react-router')
const { AIDirectorySection } = await import('./ai-directory-section')
const { readNativeUsersScope } = await import('@/features/native-users/access')
const { readManagedDirectory, saveManagedDirectory } =
  await import('@/features/ai-directory/managed-links')
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })

function signIn(role: number) {
  useAuthStore.getState().auth.setBundle({
    user: { id: 41, username: 'fixture-owner', role },
    access_token: 'synthetic',
    token_type: 'Bearer',
    access_expires_at: 9999999999,
    session: {
      sid: 'fixture-session',
      current: true,
      login_method: 'password',
      ip: null,
      user_agent: null,
      created_at: 1,
      last_active_at: 1,
      expires_at: 9999999999,
    },
  })
}

const tick = () => new Promise((resolve) => setTimeout(resolve, 10))
async function waitFor(condition: () => boolean) {
  for (let i = 0; i < 100 && !condition(); i++) {
    await act(async () => {
      await tick()
    })
  }
  assert.ok(condition(), 'provider status did not update')
}

const fixture = {
  id: 'sample',
  name: 'Original website',
  url: 'https://directory.example/about',
  category: 'resources' as const,
  summary: 'Fixture',
  description: '',
  enabled: true,
}
test('admin edits a curated link using its community endpoint and verifies the persisted list after remount', async () => {
  const host = document.body.appendChild(document.createElement('div'))
  let root = createRoot(host)
  const client = new QueryClient()
  const old = api.defaults.adapter
  let saved = [fixture]
  const calls: string[] = []
  api.defaults.adapter = async (cfg) => {
    calls.push([cfg.method, cfg.url].join(' '))
    if (cfg.method === 'put') {
      assert.equal(cfg.url, '/api/ai-directory/links')
      const body = JSON.parse(cfg.data)
      assert.deepEqual(Object.keys(body), ['links'])
      saved = body.links
    } else {
      assert.equal(cfg.url, '/api/ai-directory')
    }
    return {
      config: cfg,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: { success: true, data: { links: saved } },
    }
  }
  const render = async () => {
    const router = createRouter({
      routeTree: createRootRoute({ component: AIDirectorySection }),
      history: createMemoryHistory({ initialEntries: ['/'] }),
    })
    await act(async () => {
      root.render(
        <QueryClientProvider client={client}>
          <I18nextProvider i18n={i18n}>
            <RouterProvider router={router} />
          </I18nextProvider>
        </QueryClientProvider>
      )
    })
  }
  const button = (name: string) =>
    Array.from(host.querySelectorAll('button')).find((el) =>
      el.textContent?.includes(name)
    )
  try {
    signIn(ROLE.ADMIN)
    await render()
    await waitFor(() => Boolean(button('Original website')))
    await act(async () => {
      const edit = button('Original website')
      assert.ok(edit)
      edit.click()
    })
    const input = host.querySelector<HTMLInputElement>('#directory-name-sample')
    assert.ok(input)
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(
        HTMLInputElement.prototype,
        'value'
      )?.set
      assert.ok(setter)
      setter.call(input, 'Saved website')
      input.dispatchEvent(new Event('input', { bubbles: true }))
    })
    assert.equal(button('Save websites')?.disabled, false)
    await act(async () => {
      const save = button('Save websites')
      assert.ok(save)
      save.click()
    })
    await waitFor(
      () => calls.length === 4 && button('Save websites')?.disabled === true
    )
    assert.deepEqual(calls, [
      'get /api/ai-directory',
      'get /api/ai-directory',
      'put /api/ai-directory/links',
      'get /api/ai-directory',
    ])
    assert.equal(saved[0].name, 'Saved website')
    await act(async () => root.unmount())
    client.clear()
    root = createRoot(host)
    await render()
    await waitFor(() => Boolean(button('Saved website')))
    assert.equal(calls.length, 5)
  } finally {
    await act(async () => root.unmount())
    host.remove()
    client.clear()
    api.defaults.adapter = old
  }
})

test('malformed catalog data, stale reads and unsigned sessions cannot replace the published directory', async () => {
  signIn(ROLE.ADMIN)
  const scope = readNativeUsersScope()
  assert.ok(scope)
  const old = api.defaults.adapter
  let rows: unknown = null
  let writes = 0
  api.defaults.adapter = async (cfg) => {
    if (cfg.method === 'put') writes++
    return {
      config: cfg,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: { success: true, data: { links: rows } },
    }
  }
  try {
    assert.ok((await readManagedDirectory(scope)).length > 0)
    rows = []
    assert.deepEqual(await readManagedDirectory(scope), [])
    rows = [{ id: 'broken' }]
    await assert.rejects(readManagedDirectory(scope))
    rows = [{ ...fixture, name: 'Other admin' }]
    await assert.rejects(
      saveManagedDirectory(scope, [fixture], []),
      /Directory changed/
    )
    signIn(ROLE.USER)
    await assert.rejects(
      saveManagedDirectory(scope, [fixture], []),
      /Session changed/
    )
    assert.equal(writes, 0)
  } finally {
    api.defaults.adapter = old
  }
})
