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
const { ApiKeysDialogs } = await import('./api-keys-dialogs')
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
const originalPost = api.post
const originalPut = api.put
const originalDelete = api.delete
let cleanup: (() => Promise<void>) | undefined
afterEach(async () => {
  await cleanup?.()
  cleanup = undefined
  api.get = originalGet
  api.post = originalPost
  api.put = originalPut
  api.delete = originalDelete
  document.body.replaceChildren()
})
after(() => dom.close())
const flush = () => new Promise((resolve) => setTimeout(resolve, 25))
async function mount(search = '', includeDialogs = false) {
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
        {includeDialogs && <ApiKeysDialogs />}
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

for (const [action, layout, oneTime] of [
  ['Configure', 'desktop', true],
  ['Delete', 'mobile', false],
] as const) {
  test(`the ${layout} saved-key menu opens ${action} without retrieving a secret`, async () => {
    const key = { ...recoveredKey, one_time_reveal: oneTime }
    const posts: string[] = []
    const deletes: string[] = []
    const puts: unknown[] = []
    let holdDetailRefresh: Promise<void> | undefined
    api.get = (async (url: string) => {
      let data: unknown
      if (url.startsWith('/api/token/?')) data = { items: [key], total: 1 }
      else if (url === '/api/token/9') {
        await holdDetailRefresh
        data = { ...key }
      } else if (url === '/api/user/self/groups') {
        data = { default: { desc: 'Default', ratio: 1 } }
      } else if (url === '/api/token/auto-groups') {
        data = { groups: ['default'], max_count: 3 }
      } else if (url === '/api/user/models') data = []
      else if (url === '/api/status' || url === '/api/user/self') data = {}
      else throw new Error(`Unexpected GET ${url}`)
      return { data: { success: true, data } }
    }) as typeof api.get
    api.post = (async (url: string) => {
      posts.push(url)
      return { data: { success: false } }
    }) as typeof api.post
    api.put = (async (url: string, body: { id: number; status: number }) => {
      puts.push({ url, body })
      key.status = body.status
      return { data: { success: true } }
    }) as typeof api.put
    api.delete = (async (url: string) => {
      deletes.push(url)
      return { data: { success: true } }
    }) as typeof api.delete
    dom.happyDOM.setViewport({ width: layout === 'mobile' ? 390 : 1280 })
    const { container } = await mount('', true)
    assert.doesNotMatch(
      container.innerHTML,
      /sk-synthetic|Copy API key|Reveal API key/i
    )
    const menus = [
      ...container.querySelectorAll<HTMLButtonElement>(
        'button[aria-label="Open menu"]'
      ),
    ]
    const menu = menus.find(
      (node) => Boolean(node.closest('tbody')) === (layout === 'desktop')
    )
    assert.ok(menu)
    await act(async () => menu.click())
    const actions = [
      ...document.querySelectorAll<HTMLElement>('[role="menuitem"]'),
    ]
    assert.deepEqual(
      actions.map((node) => node.textContent?.trim()),
      ['Configure', 'Delete']
    )
    const item = actions.find((node) => node.textContent?.trim() === action)
    assert.ok(item)
    await act(async () => {
      item.click()
      await flush()
    })
    await act(async () => {
      await flush()
    })
    if (action === 'Configure') {
      const drawer = document.querySelector('[role="dialog"]')
      assert.ok(drawer)
      assert.ok(drawer.textContent?.includes('Update API Key'))
      const name = drawer.querySelector<HTMLInputElement>(
        'input[value="Recovered key"]'
      )
      assert.ok(name)
      await act(async () => {
        const setValue = Object.getOwnPropertyDescriptor(
          HTMLInputElement.prototype,
          'value'
        )?.set
        assert.ok(setValue)
        setValue.call(name, 'Edited draft')
        name.dispatchEvent(new Event('input', { bubbles: true }))
      })
      let allowRefresh!: () => void
      holdDetailRefresh = new Promise<void>((resolve) => {
        allowRefresh = resolve
      })
      await act(async () => {
        button(drawer, 'Disable').click()
        await flush()
      })
      assert.deepEqual(puts, [
        { url: '/api/token/?status_only=true', body: { id: 9, status: 2 } },
      ])
      assert.ok(button(drawer, 'Enable').disabled)
      await act(async () => {
        allowRefresh()
        await flush()
      })
      assert.equal(name.value, 'Edited draft')
      await act(async () => {
        button(drawer, 'Enable').click()
        await flush()
      })
      assert.deepEqual(puts, [
        { url: '/api/token/?status_only=true', body: { id: 9, status: 2 } },
        { url: '/api/token/?status_only=true', body: { id: 9, status: 1 } },
      ])
      assert.equal(name.value, 'Edited draft')
      assert.ok(button(drawer, 'Disable'))
    } else {
      const dialog = document.querySelector('[role="alertdialog"]')
      assert.ok(dialog)
      assert.ok(
        dialog.textContent?.includes(
          'This will permanently delete API key Recovered key'
        )
      )
      assert.deepEqual(deletes, [])
      await act(async () => {
        button(dialog, 'Delete').click()
        await flush()
      })
      assert.deepEqual(deletes, ['/api/token/9/'])
    }
    assert.deepEqual(posts, [])
  })
}

test('batch copy is available only when selection includes a retrievable key', async () => {
  dom.happyDOM.setViewport({ width: 1280 })
  const keys = [
    { ...recoveredKey, id: 9, one_time_reveal: true },
    {
      ...recoveredKey,
      id: 10,
      name: 'Legacy batch key',
      one_time_reveal: false,
    },
  ]
  api.get = (async (url: string) => {
    const data = url.startsWith('/api/token/?')
      ? { items: keys, total: keys.length }
      : {}
    return { data: { success: true, data } }
  }) as typeof api.get
  const calls: unknown[] = []
  api.post = (async (url: string, body: unknown) => {
    calls.push({ url, body })
    return {
      data: { success: true, data: { keys: { 10: 'sk-synthetic-batch' } } },
    }
  }) as typeof api.post
  const { container } = await mount()
  const rows = () =>
    container.querySelectorAll<HTMLElement>('[aria-label="Select row"]')
  assert.equal(rows().length, 2)
  await act(async () => rows()[0].click())
  const selector = '[aria-label="Copy selected keys"]'
  assert.equal(document.querySelector(selector), null)
  await act(async () => rows()[1].click())
  const copy = document.querySelector<HTMLElement>(selector)
  assert.ok(copy)
  await act(async () => {
    copy.click()
    await flush()
  })
  assert.deepEqual(calls, [
    { url: '/api/token/batch/keys', body: { ids: [10] } },
  ])
})
