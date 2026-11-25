/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { Window } from 'happy-dom'

const dom = new Window({ url: 'https://plugins.example' })
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
const { PaymentProviderStatus } = await import('./provider-status')
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

test('owner can distinguish channel availability from failed reads and retry without creating orders', async () => {
  const container = document.body.appendChild(document.createElement('div'))
  const root = createRoot(container)
  const client = new QueryClient()
  const oldAdapter = api.defaults.adapter
  const methods: string[] = []
  let unavailable = true
  let channels: Record<string, string>[] = []
  api.defaults.adapter = async (config) => {
    methods.push(config.method ?? '')
    assert.equal(config.url, '/api/user/payments/channels')
    if (unavailable) throw new Error('fixture upstream unavailable')
    return {
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: {
        success: true,
        data: { available: channels.length > 0, channels },
      },
    }
  }
  const values = () =>
    Array.from(container.querySelectorAll('dd'), (row) => row.textContent)
  const refresh = async () => {
    const button = container.querySelector('button')
    assert.ok(button)
    await act(async () => {
      button.click()
    })
  }
  try {
    signIn(ROLE.ADMIN)
    await act(async () => {
      root.render(
        <I18nextProvider i18n={i18n}>
          <QueryClientProvider client={client}>
            <PaymentProviderStatus />
          </QueryClientProvider>
        </I18nextProvider>
      )
    })
    assert.equal(container.querySelector('section'), null)
    assert.equal(methods.length, 0)

    await act(async () => {
      signIn(ROLE.SUPER_ADMIN)
    })
    await waitFor(() => container.querySelector('[role="alert"]') !== null)
    assert.deepEqual(
      Array.from(container.querySelectorAll('dt'), (row) => row.textContent),
      ['EPay', 'Stripe', 'Creem', 'Waffo', 'Waffo Pancake', 'Coinbase']
    )
    assert.deepEqual(values(), Array(6).fill('Unknown'))
    assert.equal(container.querySelector('button')?.textContent, 'Retry')

    unavailable = false
    channels = [
      {
        id: 'stripe-test',
        provider: 'stripe',
        environment: 'test',
        currency: 'USD',
      },
      {
        id: 'epay-wx',
        provider: 'epay',
        environment: 'live',
        currency: 'CNY',
        payment_type: 'wxpay',
      },
      {
        id: 'epay-ali',
        provider: 'epay',
        environment: 'live',
        currency: 'CNY',
        payment_type: 'alipay',
      },
    ]
    await refresh()
    await waitFor(() => container.textContent?.includes('stripe-test') === true)
    assert.deepEqual(values(), [
      '#epay-wx · CNY · Live · wxpay#epay-ali · CNY · Live · alipay',
      '#stripe-test · USD · Test',
      ...Array(4).fill('Not available'),
    ])

    unavailable = true
    await refresh()
    await waitFor(() => container.querySelector('[role="alert"]') !== null)
    assert.deepEqual(values(), Array(6).fill('Unknown'))

    unavailable = false
    channels = []
    await refresh()
    await waitFor(() => values()[0] === 'Not available')
    assert.deepEqual(values(), Array(6).fill('Not available'))
    assert.equal(container.querySelector('[role="alert"]'), null)
    assert.deepEqual(methods, ['get', 'get', 'get', 'get'])

    await act(async () => {
      signIn(ROLE.USER)
    })
    assert.equal(container.querySelector('section'), null)
  } finally {
    await act(async () => root.unmount())
    container.remove()
    client.clear()
    api.defaults.adapter = oldAdapter
  }
})
