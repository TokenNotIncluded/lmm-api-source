/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, test } from 'node:test'

import { Window } from 'happy-dom'

const dom = new Window({
  url: 'https://synthetic.example/wallet',
  width: 390,
  height: 844,
})
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
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { NativeWalletSubscriptions } = await import('./native-wallet')
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
const flush = () => new Promise((resolve) => setTimeout(resolve, 10))
after(() => dom.close())
function login(user = 52, sid = 'synthetic-wallet-session') {
  useAuthStore.getState().auth.setBundle({
    user: { id: user, role: 1, username: 'synthetic' },
    access_token: 'synthetic-access-token',
    token_type: 'Bearer',
    access_expires_at: 9999999999,
    session: {
      sid,
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
async function click(label: string) {
  const button = Array.from(document.querySelectorAll('button')).find(
    (el) => el.textContent?.trim() === label
  )
  assert.ok(button, label)
  await act(async () => {
    button.click()
    await flush()
  })
}

test('wallet purchase requires preview and confirmation, preserves unknown commit through remount, and isolates account switch', async () => {
  const adapter = api.defaults.adapter
  const auth = useAuthStore.getState().auth
  dom.sessionStorage.clear()
  login()
  const container = document.body.appendChild(document.createElement('div'))
  let root = createRoot(container)
  const plan = {
    id: 4,
    revision: 1,
    archived: false,
    terms: {
      title: 'Synthetic Plan',
      price_units: '100000',
      duration_seconds: 3600,
      reset_period: 'grant',
      reset_seconds: 0,
      limit_units: '1000',
      models: ['*'],
      groups: ['*'],
      max_purchases_per_user: 0,
      enabled: true,
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
  let attempts = 0
  api.defaults.adapter = async (config) => {
    let data: unknown
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
      const recovery = JSON.parse(
        dom.sessionStorage.getItem(
          'lmm-commerce:52:synthetic-wallet-session:subscription'
        )!
      )
      assert.ok(recovery, 'persist before every financial dispatch')
      writes.push(config.url!)
      if (config.url?.endsWith('/quotes')) {
        assert.equal(
          recovery.request.operation_key,
          JSON.parse(String(config.data)).operation_key
        )
        data = quote
      } else {
        assert.equal(recovery.confirmed, true)
        attempts++
        if (attempts === 1) throw new Error('acknowledgement lost')
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
  const render = async () => {
    await act(async () => {
      root.render(
        <I18nextProvider i18n={i18n}>
          <NativeWalletSubscriptions />
        </I18nextProvider>
      )
      await flush()
      await flush()
    })
  }
  try {
    await render()
    assert.equal(writes.length, 0)
    await click('Preview purchase')
    assert.equal(writes.length, 1)
    assert.equal(attempts, 0)
    assert.ok(document.body.textContent?.includes('100,000 Core credits'))
    await click('Confirm')
    assert.equal(attempts, 1)
    assert.ok(document.body.textContent?.includes('Result unknown.'))
    await act(async () => root.unmount())
    root = createRoot(container)
    await render()
    assert.equal(attempts, 1, 'mount must not retry a debit')
    await click('Retry saved request')
    assert.equal(attempts, 2)
    assert.equal(writes[1], writes[2])
    assert.equal(
      dom.sessionStorage.getItem(
        'lmm-commerce:52:synthetic-wallet-session:subscription'
      ),
      null
    )
    assert.ok(
      document.body.textContent?.includes('Subscription purchased successfully')
    )
    await act(async () => {
      login(53, 'replacement-wallet-session')
      await flush()
    })
    assert.ok(
      !document.body.textContent?.includes(
        'Subscription purchased successfully'
      )
    )
    assert.equal(attempts, 2)
  } finally {
    await act(async () => root.unmount())
    container.remove()
    api.defaults.adapter = adapter
    useAuthStore.setState({ auth })
  }
})
