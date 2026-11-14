/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, test } from 'node:test'

import type { InternalAxiosRequestConfig } from 'axios'
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
  'HTMLFormElement',
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
const { CommerceFailure } = await import('@/features/commerce/native-client')
const {
  creemOrderSchema,
  creemViewSchema,
  creemPendingSchema,
  prepareCreemOrder,
  readCreemOrder,
  freezeCreemOrder,
  navigateCreemCheckout,
} = await import('./native-creem-api')
const { NativeCreemSubscriptions } = await import('./native-creem')
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
after(() => dom.close())
const flush = () => new Promise((resolve) => setTimeout(resolve, 10))
const scope = { userId: 52, sessionId: 'synthetic-creem-session' }
const area = 'lmm-commerce:52:synthetic-creem-session:subscription-creem'
const id = `sub_${'a'.repeat(64)}`
const plan = {
  id: 4,
  revision: 1,
  archived: false,
  terms: {
    title: 'USD cash-only',
    price_units: '738772',
    duration_seconds: 0,
    duration_months: 1,
    reset_period: 'grant' as const,
    reset_seconds: 0,
    limit_units: '745116',
    models: ['*'],
    groups: ['*'],
    max_purchases_per_user: 0,
    enabled: true,
    allow_balance_pay: false,
  },
}
const price = {
  plan_id: '4',
  plan_revision: '1',
  channel_id: '5003',
  currency: 'USD',
  amount_minor: '990',
  provider_price_id: 'prod_synthetic',
  interval: 'month',
  interval_count: '1',
  success_url: 'https://synthetic.example/subscriptions',
  cancel_url: 'https://synthetic.example/subscriptions',
}
const order = () => ({
  order: {
    order_id: id,
    account_id: '9',
    plan_id: '4',
    plan_revision: '1',
    payment_channel_id: '5003',
    currency: 'USD',
    amount_minor: '990',
    expires_at_unix: String(Math.floor(Date.now() / 1000) + 1800),
    confirmed_at_unix: null as string | null,
    canceled_at_unix: null,
    proof_id: '',
    subscription_id: null as string | null,
    granted_at_unix: null as string | null,
    ends_at_unix: null,
    plan_json: Buffer.from(JSON.stringify(plan)).toString('base64'),
  },
  price,
  merchant: 'synthetic-merchant',
  environment: 'test',
  provider_subscription_id: '',
  provider_customer_id: '',
  current_period_start: '0',
  current_period_end: '0',
  state_created: '0',
  state: 'created',
  latest_grant_id: null as string | null,
})
const checkout = {
  id: 'synthetic-checkout',
  url: 'https://checkout.creem.io/pay/synthetic',
  expires_at: String(Math.floor(Date.now() / 1000) + 2700),
}
const pending = () => ({
  provider: 'creem' as const,
  operation_key: 'synthetic-creem-operation',
  plan_id: 4,
  channel_id: 5003,
  confirm_attempted: false,
})
function login(user = 52, sid = scope.sessionId) {
  useAuthStore.getState().auth.setBundle({
    user: { id: user, role: 1, username: 'synthetic' },
    access_token: 'synthetic-access',
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
function button(label: string, within: ParentNode = document) {
  const value = Array.from(within.querySelectorAll('button')).find(
    (el) => el.textContent?.trim() === label
  )
  assert.ok(value, label)
  return value
}
async function click(label: string) {
  await act(async () => {
    button(label).click()
    await flush()
    await flush()
  })
}
function response(config: InternalAxiosRequestConfig, data: unknown) {
  return { config, data, status: 200, statusText: 'OK', headers: {} }
}

test('Creem boundary validates nested replies, frozen price and hosted URL', () => {
  const raw = order()
  for (const currency of ['CNY', 'EUR']) {
    assert.ok(
      creemOrderSchema.safeParse({
        ...raw,
        order: { ...raw.order, currency },
        price: { ...price, currency },
      }).success
    )
  }
  assert.equal(
    creemOrderSchema.safeParse({
      ...raw,
      price: { ...price, provider_price_id: 'price_wrong_provider' },
    }).success,
    false
  )
  assert.equal(
    creemOrderSchema.safeParse({
      ...raw,
      price: { ...price, interval: 'week' },
    }).success,
    false
  )
  assert.equal(
    creemOrderSchema.safeParse({
      ...raw,
      price: { ...price, interval_count: '2' },
    }).success,
    false
  )
  assert.equal(
    creemPendingSchema.safeParse({ ...pending(), provider: 'stripe' }).success,
    false
  )
  assert.equal(creemOrderSchema.parse(raw).price.channel_id, 5003)
  assert.equal(
    creemOrderSchema.safeParse({
      ...raw,
      price: { ...price, plan_id: '9007199254740993' },
    }).success,
    false
  )
  assert.equal(
    creemOrderSchema.safeParse({
      ...raw,
      price: { ...price, amount_minor: '991' },
    }).success,
    false
  )
  assert.equal(
    creemOrderSchema.safeParse({
      ...raw,
      price: { ...price, amount_minor: 'bad-present' },
    }).success,
    false
  )
  assert.equal(
    creemOrderSchema.safeParse({
      ...raw,
      order: { ...raw.order, plan_revision: '2' },
    }).success,
    false
  )
  assert.equal(creemViewSchema.safeParse(raw).success, false)
  for (const url of [
    'https://user:secret@pay.synthetic.example/',
    'javascript:alert(1)',
  ]) {
    assert.equal(
      creemViewSchema.safeParse({
        order: raw,
        checkout: { ...checkout, url },
      }).success,
      false
    )
  }
  assert.equal(
    creemPendingSchema.safeParse({ ...pending(), provider: 'epay' }).success,
    false
  )
})
test('PCR dispatch binds provider route/string IDs, immutable price/merchant and current session', async () => {
  login()
  const previous = api.defaults.adapter
  let calls = 0
  const saved = {
    ...pending(),
    order_id: id,
    frozen: freezeCreemOrder(creemOrderSchema.parse(order())),
  }
  api.defaults.adapter = async (config) => {
    calls++
    assert.equal(config.authScope?.sessionId, scope.sessionId)
    assert.equal(config.skipAuthRefresh, true)
    assert.ok(
      config.url?.startsWith('/api/user/payments/subscriptions/creem/orders')
    )
    if (config.method === 'post') {
      assert.deepEqual(JSON.parse(String(config.data)), {
        plan_id: '4',
        channel_id: '5003',
      })
      assert.equal(
        config.headers.get('Idempotency-Key'),
        pending().operation_key
      )
      return response(config, order())
    }
    return response(config, {
      order: { ...order(), merchant: 'changed-merchant' },
      checkout: null,
    })
  }
  try {
    await prepareCreemOrder(scope, pending())
    await assert.rejects(readCreemOrder(scope, saved), CommerceFailure)
    login(53, 'replacement-session')
    await assert.rejects(readCreemOrder(scope, saved), CommerceFailure)
    assert.equal(calls, 2)
  } finally {
    api.defaults.adapter = previous
  }
})
for (const confirmedBeforeLoss of [false, true]) {
  test(`Creem UI recovery with prior confirmed=${confirmedBeforeLoss}; mount reads only`, async () => {
    login()
    dom.sessionStorage.clear()
    const previous = api.defaults.adapter
    const container = document.createElement('div')
    document.body.appendChild(container)
    let root = createRoot(container)
    let prepares = 0,
      confirms = 0,
      reads = 0,
      prepareKey = ''
    let raw = order()
    let hosted: typeof checkout | null = null
    api.defaults.adapter = async (config) => {
      const url = config.url ?? ''
      if (url.endsWith('/creem-prices')) {
        return response(config, { prices: [price] })
      }
      if (url.endsWith('/epay-prices')) return response(config, { prices: [] })
      if (url.endsWith('/plans')) {
        return response(config, {
          contract: 'core.subscriptions.v1',
          plans: [plan],
          payment_methods: [],
        })
      }
      if (url.endsWith('/purchases')) {
        return response(config, {
          contract: 'core.subscriptions.v1',
          purchases: [
            {
              id: 3,
              subscription_id: 7,
              plan,
              starts_at: 1,
              ends_at: 9999999999,
              used_units: '0',
              reserved_units: '0',
              entitlement_active: true,
              renewal_canceled: false,
              canceled_at: null,
              refund_journal_id: null,
            },
          ],
        })
      }
      assert.ok(url.startsWith('/api/user/payments/subscriptions/creem/orders'))
      const stored = dom.sessionStorage.getItem(area)
      assert.ok(stored)
      const recovery = JSON.parse(stored)
      if (config.method === 'post' && url.endsWith('/orders')) {
        prepares++
        assert.deepEqual(JSON.parse(String(config.data)), {
          plan_id: '4',
          channel_id: '5003',
        })
        if (prepares === 1) {
          prepareKey = String(config.headers.get('Idempotency-Key'))
          throw new Error('prepare ACK lost')
        }
        assert.equal(config.headers.get('Idempotency-Key'), prepareKey)
        assert.equal(recovery.operation_key, prepareKey)
        return response(config, raw)
      }
      if (config.method === 'post') {
        assert.ok(url.endsWith(`/${id}/confirm`))
        assert.equal(recovery.confirm_attempted, true)
        confirms++
        if (confirms !== 1 || confirmedBeforeLoss) {
          raw = {
            ...raw,
            order: { ...raw.order, confirmed_at_unix: '1700000000' },
          }
        }
        if (confirms === 1) throw new Error('checkout creation ACK lost')
        hosted = checkout
        return response(config, { order: raw, checkout: hosted })
      }
      reads++
      return response(config, { order: raw, checkout: hosted })
    }
    const render = async () =>
      act(async () => {
        root.render(
          <I18nextProvider i18n={i18n}>
            <NativeCreemSubscriptions
              scope={scope}
              plans={[plan]}
              blocked={false}
              onBlockedChange={() => {}}
            />
          </I18nextProvider>
        )
        await flush()
        await flush()
      })
    const labels = () =>
      Array.from(document.querySelectorAll('button')).map((b) =>
        b.textContent?.trim()
      )
    try {
      await render()
      assert.ok(!labels().includes('Preview purchase'))
      assert.ok(!labels().includes('Renew'))
      assert.match(document.body.textContent ?? '', /USD 9.90/)
      await click('Preview purchase · Creem')
      assert.equal(prepares, 1)
      await act(async () => root.unmount())
      root = createRoot(container)
      await render()
      assert.equal(prepares, 1)
      await click('Retry saved request')
      assert.equal(prepares, 2)
      await act(async () => {
        button('Confirm').click()
        button('Confirm').click()
        await flush()
        await flush()
      })
      assert.equal(confirms, 1)
      await act(async () => root.unmount())
      root = createRoot(container)
      await render()
      assert.equal(confirms, 1)
      await click('Retry saved request')
      assert.equal(confirms, 2)
      assert.ok(reads >= 2)
      assert.ok(!button('Continue to payment').disabled)
      const assign = Object.getOwnPropertyDescriptor(window.location, 'assign')
      const destinations: string[] = []
      Object.defineProperty(window.location, 'assign', {
        configurable: true,
        value: (url: string) => destinations.push(url),
      })
      try {
        const before = reads
        await click('Continue to payment')
        assert.ok(reads > before, 'navigation requires fresh owned read')
        assert.deepEqual(destinations, [checkout.url])
      } finally {
        if (assign) Object.defineProperty(window.location, 'assign', assign)
        else Reflect.deleteProperty(window.location, 'assign')
      }
      raw = { ...raw, latest_grant_id: '88' }
      const section = document.querySelector('section[aria-label="Creem"]')
      assert.ok(section)
      await act(async () => {
        button('Refresh', section).click()
        await flush()
        await flush()
      })
      assert.match(
        document.body.textContent ?? '',
        /Subscription purchased successfully/
      )
      assert.ok(!labels().includes('Continue to payment'))
      assert.throws(
        () =>
          navigateCreemCheckout(
            scope,
            creemViewSchema.parse({ order: raw, checkout })
          ),
        CommerceFailure
      )
      await click('Close')
      assert.equal(dom.sessionStorage.getItem(area), null)
      await act(async () => {
        login(53, 'replacement-session')
        await flush()
      })
      assert.equal(confirms, 2)
      assert.equal(prepares, 2)
    } finally {
      await act(async () => root.unmount())
      container.remove()
      api.defaults.adapter = previous
    }
  })
}

test('Creem owner price configuration sends decimal fields and rejects changed receipt', async () => {
  login()
  useAuthStore
    .getState()
    .auth.setUser({ id: 52, role: 100, username: 'synthetic' })
  const { saveAdminCreemPrice } = await import('./native-creem-admin-api')
  const { creemPriceSchema } = await import('./native-creem-api')
  const request = {
    operation_key: 'synthetic-creem-price-config',
    price: creemPriceSchema.parse(price),
  }
  const previous = api.defaults.adapter
  let changed = false
  api.defaults.adapter = async (config) => {
    assert.equal(
      config.url,
      '/core/v1/subscriptions/admin/plans/4/creem-prices'
    )
    assert.deepEqual(JSON.parse(String(config.data)), { ...request, price })
    return response(
      config,
      changed ? { ...price, provider_price_id: 'prod_changed' } : price
    )
  }
  try {
    assert.deepEqual(await saveAdminCreemPrice(scope, request), request.price)
    changed = true
    await assert.rejects(saveAdminCreemPrice(scope, request), CommerceFailure)
  } finally {
    api.defaults.adapter = previous
  }
})

test('Creem checkout copy exists in all seven locale resources', async () => {
  const { readFile } = await import('node:fs/promises')
  const keys = [
    'Billing interval',
    'Interval count',
    'Return after payment',
    'Return after cancellation',
    'Only verified paid invoices grant subscription access.',
    'Request cancellation of future renewals. Current benefits follow verified provider events.',
    'Renewal cancellation requested. Refresh to read the verified status.',
    'Cancellation scheduled',
    'Past due',
    'Unpaid',
    'Trialing',
    'Paused',
    'Incomplete expired',
  ]
  for (const language of ['en', 'fr', 'ja', 'ru', 'vi', 'zh-CN', 'zh-TW']) {
    const { translation } = JSON.parse(
      await readFile(
        new URL(`../../i18n/locales/${language}.json`, import.meta.url),
        'utf8'
      )
    )
    for (const key of keys) {
      assert.ok(
        typeof translation[key] === 'string' && translation[key].trim(),
        `${language}: ${key}`
      )
    }
  }
})

test('Creem cancellation acknowledges only 202 and reuses the owned order', async () => {
  login()
  const { cancelCreemRenewal } = await import('./native-creem-api')
  const raw = {
    ...order(),
    provider_subscription_id: 'sub_provider_original',
    state: 'active',
    latest_grant_id: '88',
  }
  const saved = {
    ...pending(),
    order_id: id,
    frozen: freezeCreemOrder(creemOrderSchema.parse(raw)),
    confirm_attempted: true,
  }
  const previous = api.defaults.adapter
  let posts = 0
  api.defaults.adapter = async (config) => {
    if (config.method === 'get') {
      return response(config, { order: raw, checkout: null })
    }
    assert.equal(
      config.url,
      `/api/user/payments/subscriptions/creem/orders/${id}/cancel-renewal`
    )
    posts++
    return { ...response(config, ''), status: 202 }
  }
  try {
    await cancelCreemRenewal(scope, saved)
    assert.equal(posts, 1)
    assert.equal((await readCreemOrder(scope, saved)).order.state, 'active')
  } finally {
    api.defaults.adapter = previous
  }
})
