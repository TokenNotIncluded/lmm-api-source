/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, test } from 'node:test'

import { AxiosError } from 'axios'
import { Window } from 'happy-dom'

const dom = new Window({ url: 'https://synthetic.example/wallet' })
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLInputElement',
  'HTMLButtonElement',
  'SVGElement',
  'Node',
  'Element',
  'Event',
  'CustomEvent',
  'MutationObserver',
  'ResizeObserver',
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
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { SubscriptionPlansCard } = await import('./subscription-plans-card')
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
after(() => dom.close())

const flush = () => new Promise((resolve) => setTimeout(resolve, 10))
async function until(check: () => boolean) {
  for (let i = 0; i < 100 && !check(); i++) await act(flush)
  assert.ok(check())
}
async function click(label: string) {
  const button = [...document.querySelectorAll('button')].find(
    (el) => el.textContent?.trim() === label
  )
  assert.ok(button, label)
  assert.equal(button.disabled, false, label)
  await act(async () => {
    button.click()
    await flush()
  })
}

test('Core plans use the native preview and saved confirmation through the wallet card', async () => {
  const adapter = api.defaults.adapter
  const auth = useAuthStore.getState().auth
  dom.sessionStorage.clear()
  useAuthStore.getState().auth.setBundle({
    user: { id: 52, role: 1, username: 'synthetic' },
    access_token: 'synthetic-access-token',
    token_type: 'Bearer',
    access_expires_at: 9999999999,
    session: {
      sid: 'synthetic-card',
      current: true,
      login_method: 'password',
      ip: null,
      user_agent: null,
      created_at: 1,
      last_active_at: 1,
      expires_at: 9999999999,
    },
  })
  const plan = {
    id: 4,
    revision: 1,
    archived: false,
    terms: {
      title: 'Synthetic native plan',
      price_units: '100000',
      duration_seconds: 3600,
      reset_period: 'grant',
      reset_seconds: 0,
      limit_units: '1000',
      models: ['*'],
      groups: ['*'],
      max_purchases_per_user: 0,
      enabled: true,
      allow_balance_pay: true,
    },
  }
  const quote = {
    id: 'f'.repeat(64),
    plan,
    starts_at: 1700000000,
    ends_at: 1700003600,
    expires_at: 1700000300,
    renew_from: null,
  }
  const writes: string[] = []
  const recoveryKey = 'lmm-commerce:52:synthetic-card:subscription'
  let confirmations = 0
  api.defaults.adapter = async (config) => {
    let data: unknown
    if (config.url?.startsWith('/api/subscription/')) {
      assert.equal(
        config.method,
        'get',
        'native plans never reach classic payment writes'
      )
      throw new AxiosError(
        'missing classic route',
        'ERR_BAD_RESPONSE',
        config,
        undefined,
        {
          config,
          status: 404,
          statusText: 'Not Found',
          headers: {},
          data: { success: false },
        }
      )
    }
    if (config.method === 'get') {
      data = config.url?.endsWith('/plans')
        ? {
            contract: 'core.subscriptions.v1',
            plans: [plan],
            payment_methods: ['wallet'],
            automatic_renewal: false,
          }
        : { contract: 'core.subscriptions.v1', purchases: [] }
    } else {
      const saved = dom.sessionStorage.getItem(recoveryKey)
      assert.ok(saved)
      const recovery = JSON.parse(saved)
      assert.ok(recovery, 'persist the original operation before dispatch')
      assert.ok(config.url)
      writes.push(config.url)
      if (config.url?.endsWith('/quotes')) {
        assert.equal(
          recovery.request.operation_key,
          JSON.parse(String(config.data)).operation_key
        )
        data = quote
      } else {
        assert.equal(
          config.url,
          `/core/v1/subscriptions/quotes/${quote.id}/confirm`
        )
        assert.equal(recovery.confirmed, true)
        confirmations++
        if (confirmations === 1) throw new Error('acknowledgement lost')
        data = {
          quote_id: quote.id,
          status: 'posted',
          purchase_id: 8,
          subscription_id: 9,
          journal_id: 10,
          amount_units: '100000',
          rejection: null,
        }
      }
    }
    return { config, data, headers: {}, status: 200, statusText: 'OK' }
  }
  const container = document.body.appendChild(document.createElement('div'))
  let root = createRoot(container)
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const render = async () => {
    await act(async () =>
      root.render(
        <QueryClientProvider client={client}>
          <I18nextProvider i18n={i18n}>
            <SubscriptionPlansCard
              topupInfo={null}
              userId={52}
              userQuota={1000000}
            />
          </I18nextProvider>
        </QueryClientProvider>
      )
    )
    await until(
      () => container.textContent?.includes('Preview purchase') === true
    )
  }
  try {
    await render()
    assert.equal(writes.length, 0)
    assert.doesNotMatch(container.textContent ?? '', /Subscribe Now/)
    await click('Preview purchase')
    assert.equal(writes.length, 1)
    assert.equal(confirmations, 0)
    await click('Confirm')
    assert.equal(confirmations, 1)
    assert.match(container.textContent ?? '', /Result unknown/)
    await act(async () => root.unmount())
    root = createRoot(container)
    await render()
    assert.equal(confirmations, 1, 'remount must not dispatch a payment')
    await click('Retry saved request')
    assert.equal(confirmations, 2)
    assert.equal(writes[1], writes[2])
    assert.equal(writes.length, 3)
    assert.equal(dom.sessionStorage.getItem(recoveryKey), null)
    assert.match(
      container.textContent ?? '',
      /Subscription purchased successfully/
    )
  } finally {
    await act(async () => root.unmount())
    client.clear()
    container.remove()
    api.defaults.adapter = adapter
    useAuthStore.setState({ auth })
  }
})
