/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.
*/
import assert from 'node:assert/strict'
import { after, afterEach, beforeEach, describe, test } from 'node:test'

import { Window } from 'happy-dom'

import type { TodoItem, TodoPage } from './api'

const domWindow = new Window({ url: 'https://console.example.test/' })
for (const key of [
  'window',
  'document',
  'navigator',
  'history',
  'location',
  'HTMLElement',
  'HTMLButtonElement',
  'HTMLTextAreaElement',
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
  'scrollTo',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: domWindow[key],
  })
}
const { act, createElement } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const {
  RouterProvider,
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
} = await import('@tanstack/react-router')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { UnifiedTodoList } = await import('./unified-todo-list')
const originalGet = api.get
const originalPost = api.post
;(
  globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })

function item(id = 1, overrides: Partial<TodoItem> = {}): TodoItem {
  return {
    id: `open_source_bounty:${id}`,
    source_id: id,
    category: 'open_source_bounty',
    type: 'comment',
    title: 'Bounty update',
    summary: `Notification ${id}`,
    read: false,
    created_at: 1_786_400_000,
    updated_at: 1_786_400_000,
    ...overrides,
  }
}
function page(overrides: Partial<TodoPage> = {}): TodoPage {
  return {
    items: [item()],
    page: 1,
    page_size: 50,
    total: 1,
    category: 'all',
    unread_count: 1,
    total_unread_count: 1,
    unread_by_category: { open_source_bounty: 1 },
    categories: [{ key: 'open_source_bounty', total: 1, unread: 1 }],
    ...overrides,
  }
}
function response(data: unknown) {
  return { data: { success: true, data } }
}
function deferred() {
  let resolve!: (value: unknown) => void
  let reject!: (reason: Error) => void
  const promise = new Promise<unknown>((yes, no) => {
    resolve = yes
    reject = no
  })
  return { promise, resolve, reject }
}
async function flush() {
  await new Promise((resolve) => setTimeout(resolve, 25))
}
async function renderList() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const rootRoute = createRootRoute({
    component: () =>
      createElement(
        QueryClientProvider,
        { client: queryClient },
        createElement(I18nextProvider, { i18n }, createElement(UnifiedTodoList))
      ),
  })
  const indexRoute = createRoute({
    getParentRoute: () => rootRoute,
    path: '/',
    component: () => null,
  })
  const usersRoute = createRoute({
    getParentRoute: () => rootRoute,
    path: 'users',
    component: () => null,
  })
  const walletRoute = createRoute({
    getParentRoute: () => rootRoute,
    path: 'wallet',
    component: () => null,
  })
  const todosRoute = createRoute({
    getParentRoute: () => rootRoute,
    path: 'todos',
    component: () => null,
  })
  const router = createRouter({
    routeTree: rootRoute.addChildren([
      indexRoute,
      usersRoute,
      walletRoute,
      todosRoute,
    ]),
    history: createMemoryHistory({ initialEntries: ['/'] }),
  })
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  await act(async () => {
    root.render(createElement(RouterProvider, { router }))
    await flush()
  })
  await act(flush)
  return {
    container,
    router,
    async close() {
      await act(async () => root.unmount())
      queryClient.clear()
      container.remove()
    },
  }
}
function button(container: HTMLElement, label: string) {
  const result = [...container.querySelectorAll('button')].find(
    (candidate) =>
      candidate.getAttribute('aria-label') === label ||
      candidate.textContent?.includes(label)
  )
  assert.ok(result, `Missing button: ${label}`)
  return result
}
async function click(container: HTMLElement, label: string) {
  await act(async () => {
    button(container, label).click()
    await flush()
  })
  await act(flush)
}
beforeEach(() => {
  useAuthStore.getState().auth.setUser({ id: 10, username: 'staff', role: 10 })
  api.get = (async () => response(page())) as typeof api.get
  api.post = (async () => response({ marked: 1 })) as typeof api.post
})
afterEach(() => {
  useAuthStore.getState().auth.reset()
  api.get = originalGet
  api.post = originalPost
  document.body.replaceChildren()
})
after(() => domWindow.close())

describe('todo feed regressions', () => {
  test('fetches later pages and resets pagination when the category changes', async () => {
    const gets: string[] = []
    api.get = (async (url: string) => {
      gets.push(url)
      const params = new URL(url, 'https://console.example.test').searchParams
      const currentPage = Number(params.get('p'))
      return response(
        page({
          page: currentPage,
          total: 51,
          category: params.get('category') as TodoPage['category'],
          items: [item(currentPage, { summary: `Page ${currentPage} result` })],
          categories: [{ key: 'open_source_bounty', total: 51, unread: 1 }],
        })
      )
    }) as typeof api.get
    const rendered = await renderList()
    try {
      await click(rendered.container, 'Next page')
      assert.ok(rendered.container.textContent?.includes('Page 2 result'))
      assert.ok(gets.some((url) => url.includes('category=all&p=2&')))
      await click(rendered.container, 'Bounty notifications')
      assert.ok(gets.at(-1)?.includes('category=open_source_bounty&p=1&'))
      assert.ok(rendered.container.textContent?.includes('Page 1 result'))
    } finally {
      await rendered.close()
    }
  })

  test('opens the destination before the read receipt resolves and catches failure', async () => {
    const read = deferred()
    api.get = (async () =>
      response(
        page({
          items: [
            item(1, {
              category: 'security_incident',
              summary: 'Review the affected user',
              details: { username: 'customer' },
            }),
          ],
        })
      )) as typeof api.get
    api.post = (() => read.promise) as typeof api.post
    const rendered = await renderList()
    try {
      await click(rendered.container, 'Review the affected user')
      assert.equal(rendered.router.state.location.pathname, '/users')
      const search = rendered.router.state.location.search
      assert.ok('filter' in search)
      assert.equal(search.filter, 'customer')
      await act(async () => {
        read.reject(new Error('Read receipt failed'))
        await flush()
      })
      assert.ok(rendered.container.textContent?.includes('Operation failed'))
      api.post = (async () => response({ marked: 1 })) as typeof api.post
      await click(rendered.container, 'Retry')
      assert.equal(rendered.container.querySelector('[role="alert"]'), null)
    } finally {
      read.resolve(response({ marked: 1 }))
      await rendered.close()
    }
  })

  test('deduplicates a fast double click without locking other unread rows', async () => {
    const first = deferred()
    const ids: number[] = []
    api.get = (async () =>
      response(page({ items: [item(1), item(2)], total: 2 }))) as typeof api.get
    api.post = ((_: string, body: { ids: number[] }) => {
      ids.push(body.ids[0])
      return body.ids[0] === 1
        ? first.promise
        : Promise.resolve(response({ marked: 1 }))
    }) as typeof api.post
    const rendered = await renderList()
    try {
      await act(async () => {
        const firstRow = button(rendered.container, 'Notification 1')
        firstRow.click()
        firstRow.click()
        await flush()
      })
      assert.equal(button(rendered.container, 'Notification 2').disabled, false)
      await click(rendered.container, 'Notification 2')
      assert.deepEqual(ids, [1, 2])
      await act(async () => {
        first.resolve(response({ marked: 1 }))
        await flush()
      })
    } finally {
      first.resolve(response({ marked: 1 }))
      await rendered.close()
    }
  })

  test('retains existing rows when a background refresh fails', async () => {
    const rendered = await renderList()
    try {
      api.get = (async () => {
        throw new Error('Temporary outage')
      }) as typeof api.get
      await click(rendered.container, 'Refresh')
      assert.ok(
        rendered.container.textContent?.includes('Failed to load to-dos')
      )
      assert.ok(rendered.container.textContent?.includes('Notification 1'))
      assert.equal(
        rendered.container.textContent?.includes('No pending to-dos'),
        false
      )
    } finally {
      await rendered.close()
    }
  })

  test('returns to the first page when the last page disappears', async () => {
    const gets: string[] = []
    api.get = (async (url: string) => {
      gets.push(url)
      return response(
        gets.length === 1 ? page({ total: 51 }) : page({ items: [], total: 0 })
      )
    }) as typeof api.get
    const rendered = await renderList()
    try {
      await click(rendered.container, 'Next page')
      await act(flush)
      assert.ok(gets.some((url) => url.includes('&p=2&')))
      assert.ok(gets.at(-1)?.includes('&p=1&'))
      assert.ok(rendered.container.textContent?.includes('No pending to-dos'))
    } finally {
      await rendered.close()
    }
  })

  test('renders a notification even when its timestamp is outside the Date range', async () => {
    api.get = (async () =>
      response(
        page({ items: [item(1, { updated_at: 9e15 })] })
      )) as typeof api.get
    const rendered = await renderList()
    try {
      assert.ok(rendered.container.textContent?.includes('Notification 1'))
      assert.equal(rendered.container.querySelector('time'), null)
    } finally {
      await rendered.close()
    }
  })

  for (const change of ['account', 'session', 'role'] as const) {
    test(`does not reuse cached rows or late read errors after a ${change} change`, async () => {
      const read = deferred()
      const nextPage = deferred()
      api.post = (() => read.promise) as typeof api.post
      const rendered = await renderList()
      try {
        await click(rendered.container, 'Notification 1')
        api.get = (() => nextPage.promise) as typeof api.get
        await act(async () => {
          if (change === 'session') {
            useAuthStore.setState((state) => ({
              auth: {
                ...state.auth,
                session: {
                  sid: 'replacement-session',
                  current: true,
                  login_method: 'password',
                  ip: '',
                  user_agent: '',
                  created_at: 1,
                  last_active_at: 1,
                  expires_at: 2,
                },
              },
            }))
          } else {
            useAuthStore.getState().auth.setUser({
              id: change === 'account' ? 11 : 10,
              username: 'replacement',
              role: change === 'role' ? 1 : 10,
            })
          }
          await flush()
        })
        assert.equal(
          rendered.container.textContent?.includes('Notification 1'),
          false
        )
        await act(async () => {
          read.reject(new Error('Old request failed'))
          nextPage.resolve(response(page({ items: [item(2)] })))
          await flush()
        })
        assert.ok(rendered.container.textContent?.includes('Notification 2'))
        assert.equal(rendered.container.querySelector('[role="alert"]'), null)
      } finally {
        read.resolve(response({ marked: 1 }))
        nextPage.resolve(response(page({ items: [] })))
        await rendered.close()
      }
    })
  }
})

function lowBalanceItem(overrides: Partial<TodoItem> = {}): TodoItem {
  return item(91, {
    id: 'notification:91',
    category: 'account_action',
    type: 'low_balance',
    title: 'Low balance',
    summary: 'Your wallet balance is below your notification threshold.',
    details: {
      balance_units: '-9007199254740993',
      threshold_units: '9007199254740991',
      unit: 'credit_500k_usd',
    },
    ...overrides,
  })
}

describe('low balance notifications', () => {
  for (const role of [1, 10]) {
    for (const read of [false, true]) {
      test(`role ${role} opens the owner wallet from a ${read ? 'read' : 'unread'} notice`, async () => {
        useAuthStore
          .getState()
          .auth.setUser({ id: 20, username: 'owner', role })
        const notice = lowBalanceItem({ read })
        const gets: string[] = []
        const posts: Array<{ url: string; body: unknown }> = []
        api.get = (async (url: string) => {
          gets.push(url)
          return response(page({ items: [notice] }))
        }) as typeof api.get
        api.post = (async (url: string, body: unknown) => {
          posts.push({ url, body })
          return response({ marked: 1 })
        }) as typeof api.post
        const rendered = await renderList()
        try {
          const row = button(rendered.container, 'Low balance')
          assert.equal(row.disabled, false)
          assert.equal(row.querySelector('button, a'), null)
          assert.ok(
            row.textContent?.includes('Balance (Credits): -9007199254740993')
          )
          assert.ok(
            row.textContent?.includes(
              'Balance threshold (credit units): 9007199254740991'
            )
          )
          row.focus()
          assert.equal(document.activeElement, row)
          await click(rendered.container, 'Low balance')
          assert.equal(rendered.router.state.location.pathname, '/wallet')
          assert.deepEqual(rendered.router.state.location.search, {})
          assert.deepEqual(
            posts,
            read
              ? []
              : [
                  {
                    url: '/api/todos/read',
                    body: { category: 'account_action', ids: [91], all: false },
                  },
                ]
          )
          assert.ok(gets.every((url) => url.startsWith('/api/todos?')))
        } finally {
          await rendered.close()
        }
      })
    }
  }

  for (const balance of ['0', '14999']) {
    test(`keeps the captured balance ${balance} in exact credit units`, async () => {
      api.get = (async () =>
        response(
          page({
            items: [
              lowBalanceItem({
                details: {
                  balance_units: balance,
                  threshold_units: '15000',
                  unit: 'credit_500k_usd',
                },
              }),
            ],
          })
        )) as typeof api.get
      const rendered = await renderList()
      try {
        assert.ok(
          button(rendered.container, 'Low balance').textContent?.includes(
            `Balance (Credits): ${balance}`
          )
        )
      } finally {
        await rendered.close()
      }
    })
  }

  for (const details of [
    undefined,
    { balance_units: '7', threshold_units: '8', unit: 'USD' },
    { balance_units: 7, threshold_units: 8, unit: 'credit_500k_usd' },
    { balance_units: '1e3', threshold_units: '-1', unit: 'credit_500k_usd' },
    { balance_units: '-0', threshold_units: '01', unit: 'credit_500k_usd' },
  ]) {
    test(`shows unknown values for invalid low-balance details: ${JSON.stringify(details)}`, async () => {
      useAuthStore
        .getState()
        .auth.setUser({ id: 20, username: 'owner', role: 1 })
      api.get = (async () =>
        response(
          page({ items: [lowBalanceItem({ details })] })
        )) as typeof api.get
      const rendered = await renderList()
      try {
        const row = button(rendered.container, 'Low balance')
        assert.ok(row.textContent?.includes('Balance (Credits): Unknown'))
        assert.ok(
          row.textContent?.includes('Balance threshold (credit units): Unknown')
        )
        await click(rendered.container, 'Low balance')
        assert.equal(rendered.router.state.location.pathname, '/wallet')
        assert.deepEqual(rendered.router.state.location.search, {})
      } finally {
        await rendered.close()
      }
    })
  }

  test('opens the wallet while the read receipt is pending, and keeps retry available on failure', async () => {
    useAuthStore.getState().auth.setUser({ id: 20, username: 'owner', role: 1 })
    const read = deferred()
    api.get = (async () =>
      response(page({ items: [lowBalanceItem()] }))) as typeof api.get
    api.post = (() => read.promise) as typeof api.post
    const rendered = await renderList()
    try {
      await click(rendered.container, 'Low balance')
      assert.equal(rendered.router.state.location.pathname, '/wallet')
      await act(async () => {
        read.reject(new Error('Read receipt failed'))
        await flush()
      })
      assert.ok(rendered.container.textContent?.includes('Operation failed'))
      api.post = (async () => response({ marked: 1 })) as typeof api.post
      await click(rendered.container, 'Retry')
      assert.equal(rendered.container.querySelector('[role="alert"]'), null)
    } finally {
      read.resolve(response({ marked: 1 }))
      await rendered.close()
    }
  })

  for (const role of [1, 10]) {
    test(`role ${role} retains the existing account request destination policy`, async () => {
      useAuthStore.getState().auth.setUser({ id: 20, username: 'owner', role })
      api.get = (async () =>
        response(
          page({
            items: [
              item(17, {
                category: 'account_action',
                type: 'request',
                title: 'account_action.request',
                summary: 'Review account request',
              }),
            ],
          })
        )) as typeof api.get
      const rendered = await renderList()
      try {
        await click(rendered.container, 'Review account request')
        assert.equal(
          rendered.router.state.location.pathname,
          role === 10 ? '/todos' : '/'
        )
        assert.deepEqual(
          rendered.router.state.location.search,
          role === 10 ? { todo: 'account_action', request: 17 } : {}
        )
        assert.equal(
          rendered.container.textContent?.includes('Balance (Credits):'),
          false
        )
      } finally {
        await rendered.close()
      }
    })
  }
})
