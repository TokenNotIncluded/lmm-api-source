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
      duration_seconds: 0,
      duration_months: 1,
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
    assert.match(document.body.textContent ?? '', /1 months/)
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

test('renew uses the current balance-enabled catalogue plan, preserves record restrictions and still requires a persisted preview then confirmation', async () => {
  const adapter = api.defaults.adapter
  const auth = useAuthStore.getState().auth
  dom.sessionStorage.clear()
  login()
  const plan: import('./native-api').NativePlan = {
    id: 4,
    revision: 2,
    archived: false,
    terms: {
      title: 'Current Renewal Plan',
      price_units: '220000',
      duration_seconds: 0,
      duration_months: 1,
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
  const historical = {
    ...plan,
    revision: 1,
    terms: { ...plan.terms, title: 'Historical Plan', price_units: '100000' },
  }
  const purchase = {
    id: 8,
    subscription_id: 9,
    plan: historical,
    starts_at: 1700000000,
    ends_at: 1700003600,
    used_units: '0',
    reserved_units: '0',
    entitlement_active: true,
    renewal_canceled: false,
    canceled_at: null,
    refund_journal_id: null,
  }
  const purchases = [
    purchase,
    { ...purchase, id: 10, renewal_canceled: true, canceled_at: 1700000010 },
    { ...purchase, id: 11, entitlement_active: false, refund_journal_id: 12 },
  ]
  let catalogue = [plan]
  const writes: { path: string; body: Record<string, unknown> }[] = []
  const quote = {
    id: 'e'.repeat(64),
    plan,
    starts_at: 1700003600,
    ends_at: 1700007200,
    expires_at: 2000000000,
    renew_from: purchase.id,
  }
  const key = 'lmm-commerce:52:synthetic-wallet-session:subscription'
  api.defaults.adapter = async (config) => {
    assert.deepEqual(config.authScope, {
      userId: 52,
      sessionId: 'synthetic-wallet-session',
    })
    let data: unknown
    if (config.method === 'get') {
      if (config.url === '/core/v1/subscriptions/plans') {
        data = { contract: 'core.subscriptions.v1', plans: catalogue }
      } else if (config.url === '/core/v1/subscriptions/purchases') {
        data = { contract: 'core.subscriptions.v1', purchases }
      } else {
        assert.ok(
          config.url?.endsWith('/epay-prices') ||
            config.url?.endsWith('/pancake-prices')
        )
        data = { prices: [] }
      }
    } else {
      const raw = dom.sessionStorage.getItem(key)
      assert.ok(raw, 'persist before every financial dispatch')
      const saved = JSON.parse(raw)
      const body =
        typeof config.data === 'string'
          ? JSON.parse(config.data)
          : (config.data ?? {})
      assert.ok(config.url)
      writes.push({ path: config.url, body })
      if (config.url.endsWith('/quotes')) {
        assert.equal(saved.confirmed, false)
        assert.deepEqual(body, saved.request)
        assert.equal(body.plan_id, plan.id)
        assert.equal(body.renew_from, purchase.id)
        data = quote
      } else {
        assert.equal(
          config.url,
          `/core/v1/subscriptions/quotes/${quote.id}/confirm`
        )
        assert.equal(saved.confirmed, true)
        assert.equal(saved.quote.id, quote.id)
        data = {
          quote_id: quote.id,
          status: 'posted',
          purchase_id: 13,
          subscription_id: 14,
          journal_id: 15,
          amount_units: plan.terms.price_units,
          rejection: null,
        }
      }
    }
    return { config, data, headers: {}, status: 200, statusText: 'OK' }
  }
  const container = document.body.appendChild(document.createElement('div'))
  const root = createRoot(container)
  const renewButtons = () =>
    [...container.querySelectorAll('button')].filter(
      (button) => button.textContent?.trim() === 'Renew'
    )
  try {
    await act(async () => {
      root.render(
        <I18nextProvider i18n={i18n}>
          <NativeWalletSubscriptions />
        </I18nextProvider>
      )
      await flush()
      await flush()
    })
    assert.deepEqual(
      renewButtons().map((button) => button.disabled),
      [false, true, true]
    )
    catalogue = [
      { ...plan, terms: { ...plan.terms, allow_balance_pay: false } },
    ]
    await click('Refresh')
    assert.equal(renewButtons().length, 0)
    catalogue = []
    await click('Refresh')
    assert.equal(renewButtons().length, 0)
    catalogue = [plan]
    await click('Refresh')
    assert.deepEqual(
      renewButtons().map((button) => button.disabled),
      [false, true, true]
    )
    assert.equal(writes.length, 0)
    await click('Renew')
    assert.equal(writes.length, 1)
    assert.ok(container.textContent?.includes('Current Renewal Plan'))
    assert.ok(container.textContent?.includes('220,000 Core credits'))
    assert.equal(
      JSON.parse(dom.sessionStorage.getItem(key) || '{}').confirmed,
      false
    )
    await click('Confirm')
    assert.equal(writes.length, 2)
    assert.ok(
      container.textContent?.includes('Subscription purchased successfully')
    )
    assert.equal(dom.sessionStorage.getItem(key), null)
  } finally {
    await act(async () => root.unmount())
    container.remove()
    api.defaults.adapter = adapter
    useAuthStore.setState({ auth })
  }
})

test('saved purchase status recovers its receipt with GET only and preserves unknown or mismatched reads', async () => {
  const adapter = api.defaults.adapter
  const auth = useAuthStore.getState().auth
  dom.sessionStorage.clear()
  login()
  const plan = {
    id: 4,
    revision: 1,
    archived: false,
    terms: {
      title: 'Recoverable Plan',
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
    id: 'd'.repeat(64),
    plan,
    starts_at: 1700000000,
    ends_at: 1700003600,
    expires_at: 1700000300,
    renew_from: null,
  }
  const receipt = {
    quote_id: quote.id,
    status: 'posted',
    purchase_id: 8,
    subscription_id: 9,
    journal_id: 10,
    amount_units: '100000',
    rejection: null,
  }
  const key = 'lmm-commerce:52:synthetic-wallet-session:subscription'
  dom.sessionStorage.setItem(
    key,
    JSON.stringify({
      kind: 'purchase',
      request: {
        operation_key: 'saved-subscription-key',
        plan_id: plan.id,
        renew_from: null,
      },
      quote,
      confirmed: true,
    })
  )
  let mode = 'pending'
  let reads = 0
  let writes = 0
  api.defaults.adapter = async (config) => {
    if (config.method !== 'get') writes++
    assert.equal(
      config.method,
      'get',
      'status recovery must never dispatch payment'
    )
    assert.deepEqual(config.authScope, {
      userId: 52,
      sessionId: 'synthetic-wallet-session',
    })
    let data: unknown
    if (config.url === `/core/v1/subscriptions/quotes/${quote.id}`) {
      reads++
      if (mode === 'missing') {
        throw Object.assign(new Error('not found'), {
          isAxiosError: true,
          response: {
            status: 404,
            data: { error: { code: 'SUBSCRIPTION_NOT_FOUND' } },
          },
        })
      }
      data = {
        contract: 'core.subscriptions.v1',
        quote,
        receipt:
          mode === 'pending'
            ? null
            : mode === 'mismatched'
              ? { ...receipt, quote_id: 'c'.repeat(64) }
              : receipt,
      }
    } else if (config.url === '/core/v1/subscriptions/plans') {
      data = { contract: 'core.subscriptions.v1', plans: [plan] }
    } else if (config.url === '/core/v1/subscriptions/purchases') {
      data = { contract: 'core.subscriptions.v1', purchases: [] }
    } else {
      data = { prices: [] }
    }
    return { config, data, headers: {}, status: 200, statusText: 'OK' }
  }
  const container = document.body.appendChild(document.createElement('div'))
  const root = createRoot(container)
  try {
    await act(async () => {
      root.render(
        <I18nextProvider i18n={i18n}>
          <NativeWalletSubscriptions />
        </I18nextProvider>
      )
      await flush()
      await flush()
    })
    assert.equal(
      reads,
      0,
      'mount restores the operation without dispatching it'
    )
    for (mode of ['pending', 'missing', 'mismatched']) {
      await click('Refresh payment status')
      assert.ok(dom.sessionStorage.getItem(key))
      assert.match(container.textContent ?? '', /Result unknown/)
      assert.ok(
        ![...container.querySelectorAll('button')].some(
          (button) => button.textContent?.trim() === 'Cancel'
        ),
        'a failed read never releases an uncertain debit for a new operation'
      )
    }
    mode = 'posted'
    await click('Refresh payment status')
    assert.equal(reads, 4)
    assert.equal(writes, 0)
    assert.equal(dom.sessionStorage.getItem(key), null)
    assert.match(
      container.textContent ?? '',
      /Subscription purchased successfully/
    )
    assert.match(container.textContent ?? '', /100,000 Core credits/)
  } finally {
    await act(async () => root.unmount())
    container.remove()
    api.defaults.adapter = adapter
    useAuthStore.setState({ auth })
  }
})
