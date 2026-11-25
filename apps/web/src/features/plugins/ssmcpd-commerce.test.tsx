/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { Window } from 'happy-dom'

const dom = new Window({ url: 'https://traffic.invalid' })
for (const name of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'Node',
  'Element',
  'Event',
  'HTMLInputElement',
  'MutationObserver',
  'sessionStorage',
] as const) {
  Object.defineProperty(globalThis, name, {
    configurable: true,
    value: name === 'window' ? dom : dom[name],
  })
}
Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
  configurable: true,
  value: true,
})
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const {
  createMemoryHistory,
  createRootRoute,
  createRouter,
  RouterContextProvider,
} = await import('@tanstack/react-router')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { SSMCPDCommerce } = await import('./ssmcpd-commerce')

test('a lost response preserves the frozen checkout across remount and redemption only uses the original order', async () => {
  const prev = useAuthStore.getState().auth
  const adapter = api.defaults.adapter
  const state = { userId: 7, sessionId: 'buyer-a' }
  prev.setBundle({
    user: { id: 7, role: 1, username: 'fixture' },
    access_token: 'fixture-access-token',
    token_type: 'Bearer',
    access_expires_at: 9999999999,
    session: {
      sid: 'buyer-a',
      current: true,
      login_method: 'password',
      ip: null,
      user_agent: null,
      created_at: 1,
      last_active_at: 1,
      expires_at: 9999999999,
    },
  })
  const i18n = createInstance()
  await i18n
    .use(initReactI18next)
    .init({ lng: 'en', resources: { en: { translation: {} } } })
  const q = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  const router = createRouter({
    routeTree: createRootRoute(),
    history: createMemoryHistory(),
  })
  const node = document.body.appendChild(document.createElement('div'))
  let root = createRoot(node)
  const tree = (
    <QueryClientProvider client={q}>
      <I18nextProvider i18n={i18n}>
        <RouterContextProvider router={router}>
          <SSMCPDCommerce {...state} />
        </RouterContextProvider>
      </I18nextProvider>
    </QueryClientProvider>
  )
  const buyBodies: string[] = []
  const id = '6'.repeat(64)
  let grants = 0
  let price = '34'
  const status = () => ({
    order_id: id,
    credits: '34',
    bytes: '600',
    period: '2026-11',
    money_state: grants ? 'settled' : 'paid',
    grant_state: grants ? 'granted' : 'not_dispatched',
  })
  api.defaults.adapter = async (config) => {
    assert.deepEqual(config.authScope, state)
    let data
    if (config.url === '/api/plugins/ssmcpd/offer')
      data = { period: '2026-11', unit_credits: price, bytes_per_unit: '600' }
    else if (
      config.url === '/api/plugins/ssmcpd/orders' &&
      config.method === 'post'
    ) {
      buyBodies.push(config.data)
      if (buyBodies.length === 1) throw new Error('lost response')
      data = status()
    } else if (config.url === '/api/plugins/ssmcpd/orders/' + id + '/redeem') {
      assert.equal(config.method, 'post')
      assert.deepEqual(JSON.parse(config.data), {})
      grants++
      data = status()
    } else if (config.url === '/api/plugins/ssmcpd/orders/' + id)
      data = status()
    else throw new Error('unexpected URL ' + config.url)
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: { success: true, data },
    }
  }
  const waitFor = async (fn: () => boolean) => {
    for (let i = 0; i < 100 && !fn(); i++)
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 8))
      })
    assert.ok(fn())
  }
  const button = (label: string) =>
    Array.from(node.querySelectorAll('button')).find(
      (b) => b.textContent === label
    )
  const click = async (label: string) => {
    await waitFor(() => !!button(label))
    await act(async () => {
      button(label)!.click()
    })
  }
  try {
    await act(async () => {
      root.render(tree)
    })
    await click('Buy with balance')
    await waitFor(() => !!node.querySelector('[role="alert"]'))
    await act(async () => root.unmount())
    q.clear()
    price = '55'
    root = createRoot(node)
    await act(async () => {
      root.render(tree)
    })
    await click('Retry the same purchase')
    await waitFor(() => !!button('Redeem traffic'))
    assert.equal(buyBodies.length, 2)
    assert.equal(buyBodies[0], buyBodies[1])
    assert.equal(JSON.parse(buyBodies[1]).unit_credits, '34')
    await click('Redeem traffic')
    await waitFor(() => node.textContent?.includes('Granted') === true)
    assert.equal(grants, 1)
    assert.equal(buyBodies.length, 2)
  } finally {
    await act(async () => root.unmount())
    q.clear()
    node.remove()
    api.defaults.adapter = adapter
    useAuthStore.setState({ auth: prev })
    await dom.close()
  }
})
