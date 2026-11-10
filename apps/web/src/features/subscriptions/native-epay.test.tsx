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
  epayOrderSchema,
  epayPriceSchema,
  epayPendingSchema,
  freezeEPayOrder,
  prepareEPayOrder,
  readEPayOrder,
  navigateEPayCheckout,
} = await import('./native-epay-api')
const { NativeWalletSubscriptions } = await import('./native-wallet')
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
after(() => dom.close())
const flush = () => new Promise((resolve) => setTimeout(resolve, 10))
const id = `sub_${'a'.repeat(64)}`
const scope = { userId: 52, sessionId: 'synthetic-epay-session' }
const area = 'lmm-commerce:52:synthetic-epay-session:subscription-epay'
const plan = {
  id: 4,
  revision: 1,
  archived: false,
  terms: {
    title: '冻结套餐',
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
const rawOrder = () => ({
  order_id: id,
  account_id: '952',
  plan_id: '4',
  plan_revision: '1',
  payment_channel_id: '9',
  currency: 'CNY',
  amount_minor: '1234',
  expires_at_unix: '4102444800',
  confirmed_at_unix: null as string | null,
  canceled_at_unix: null as string | null,
  proof_id: '',
  subscription_id: null as string | null,
  granted_at_unix: null as string | null,
  ends_at_unix: null as string | null,
  plan_json: Buffer.from(JSON.stringify(plan), 'utf8').toString('base64'),
})
const checkout = {
  intent_id: '0' as const,
  order_id: id,
  method: 'POST' as const,
  checkout_url:
    'https://pay.synthetic.example/submit?sign=a%26b&submit=literal&item=one&item=two&title=%3Cscript%3E',
}
const pending = () => ({
  operation_key: 'synthetic-idempotency-key',
  plan_id: 4,
  channel_id: 9,
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
function button(label: string) {
  const value = Array.from(document.querySelectorAll('button')).find(
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

test('protojson boundary preserves UTF-8 frozen plan; rejects unsafe IDs, malformed plans and unsafe checkout', () => {
  assert.equal(
    epayOrderSchema.parse(rawOrder()).plan_json.terms.title,
    '冻结套餐'
  )
  const {
    confirmed_at_unix: _confirmed,
    canceled_at_unix: _canceled,
    subscription_id: _subscription,
    granted_at_unix: _grant,
    ends_at_unix: _ends,
    ...absentOptional
  } = rawOrder()
  assert.equal(epayOrderSchema.parse(absentOptional).confirmed_at_unix, null)
  assert.equal(
    epayOrderSchema.safeParse({ ...rawOrder(), account_id: 52 }).success,
    false
  )
  assert.equal(
    epayOrderSchema.safeParse({ ...rawOrder(), plan_id: '9007199254740993' })
      .success,
    false
  )
  assert.equal(
    epayOrderSchema.safeParse({ ...rawOrder(), plan_json: 'not base64!' })
      .success,
    false
  )
  assert.equal(
    epayOrderSchema.safeParse({
      ...rawOrder(),
      plan_json: Buffer.from(
        JSON.stringify({ ...plan, id: Number.MAX_SAFE_INTEGER + 1 })
      ).toString('base64'),
    }).success,
    false
  )
  assert.equal(
    epayPriceSchema.safeParse({
      plan_id: 4,
      plan_revision: 1,
      channel_id: Number.MAX_SAFE_INTEGER + 1,
      currency: 'CNY',
      amount_minor: '1',
    }).success,
    false
  )
  assert.equal(
    epayPendingSchema.safeParse({ ...pending(), confirm_attempted: true })
      .success,
    false
  )
  login()
  for (const url of [
    'javascript:alert(1)',
    'http://pay.example/',
    '//pay.example/',
    'https://user:secret@pay.example/',
    'https://pay.example/#secret',
  ]) {
    assert.throws(() =>
      navigateEPayCheckout(scope, { ...checkout, checkout_url: url })
    )
  }
})

test('prepare sends exact key/body; read rejects changed frozen terms/owner and late account response', async () => {
  login()
  const original = api.defaults.adapter
  try {
    api.defaults.adapter = async (config) => {
      assert.equal(
        config.headers.get('Idempotency-Key'),
        pending().operation_key
      )
      assert.deepEqual(JSON.parse(String(config.data)), {
        plan_id: 4,
        channel_id: 9,
      })
      return response(config, rawOrder())
    }
    const order = await prepareEPayOrder(scope, pending())
    const saved = { ...pending(), order_id: id, frozen: freezeEPayOrder(order) }
    api.defaults.adapter = async (config) =>
      response(config, { ...rawOrder(), amount_minor: '1235' })
    await assert.rejects(readEPayOrder(scope, saved), /FROZEN_ORDER_MISMATCH/)
    api.defaults.adapter = async (config) =>
      response(config, { ...rawOrder(), account_id: '53' })
    await assert.rejects(readEPayOrder(scope, saved), /FROZEN_ORDER_MISMATCH/)
    api.defaults.adapter = async (config) => {
      login(53, 'replacement')
      return response(config, rawOrder())
    }
    await assert.rejects(prepareEPayOrder(scope, pending()), /SESSION_CHANGED/)
  } finally {
    api.defaults.adapter = original
  }
})

test('GET/POST navigation preserves exact URL and duplicate fields without HTML execution; stale scope cannot navigate', () => {
  login()
  const originalSubmit = HTMLFormElement.prototype.submit
  const originalAssign = window.location.assign
  let assigned = ''
  const forms: {
    action: string
    method: string
    fields: [string, string][]
  }[] = []
  try {
    window.location.assign = (value) => {
      assigned = String(value)
    }
    navigateEPayCheckout(scope, { ...checkout, method: 'GET' })
    assert.equal(assigned, checkout.checkout_url)
    HTMLFormElement.prototype.submit = function () {
      forms.push({
        action: this.action,
        method: this.method.toUpperCase(),
        fields: Array.from(this.querySelectorAll('input')).map((input) => [
          input.name,
          input.value,
        ]),
      })
      assert.equal(this.querySelector('script'), null)
    }
    navigateEPayCheckout(scope, checkout)
    assert.deepEqual(forms, [
      {
        action: 'https://pay.synthetic.example/submit',
        method: 'POST',
        fields: [
          ['sign', 'a&b'],
          ['submit', 'literal'],
          ['item', 'one'],
          ['item', 'two'],
          ['title', '<script>'],
        ],
      },
    ])
    assert.equal(document.querySelector('form'), null)
    login(53, 'other')
    assert.throws(() => navigateEPayCheckout(scope, checkout), CommerceFailure)
    assert.equal(forms.length, 1)
  } finally {
    HTMLFormElement.prototype.submit = originalSubmit
    window.location.assign = originalAssign
  }
})

test('native wallet lost prepare reuses its key; lost confirm stays on original order without repeating confirmation', async () => {
  login()
  dom.sessionStorage.clear()
  const originalAdapter = api.defaults.adapter
  const originalSubmit = HTMLFormElement.prototype.submit
  const container = document.body.appendChild(document.createElement('div'))
  let root = createRoot(container)
  let prepares = 0,
    confirms = 0,
    posts = 0
  let originalKey = ''
  let order = rawOrder()
  const paths: string[] = []
  HTMLFormElement.prototype.submit = function () {
    posts++
    assert.equal(this.action, 'https://pay.synthetic.example/submit')
  }
  api.defaults.adapter = async (config) => {
    const url = config.url ?? ''
    paths.push(`${config.method} ${url}`)
    if (config.method === 'get') {
      if (url.endsWith('/epay-prices')) {
        return response(config, {
          prices: [
            {
              plan_id: 4,
              plan_revision: 1,
              channel_id: 9,
              currency: 'CNY',
              amount_minor: '1234',
              channel_name: 'Synthetic EPay',
            },
          ],
        })
      }
      if (url.endsWith('/plans')) {
        return response(config, {
          contract: 'core.subscriptions.v1',
          plans: [plan],
        })
      }
      if (url.endsWith('/purchases')) {
        return response(config, {
          contract: 'core.subscriptions.v1',
          purchases: [],
        })
      }
      assert.ok(url.endsWith(id))
      return response(config, order)
    }
    const raw = dom.sessionStorage.getItem(area)
    assert.ok(raw)
    const recovery = epayPendingSchema.parse(JSON.parse(raw))
    if (url.endsWith('/orders')) {
      prepares++
      const key = String(config.headers.get('Idempotency-Key'))
      if (!originalKey) originalKey = key
      assert.equal(key, originalKey)
      assert.equal(recovery.confirm_attempted, false)
      if (prepares === 1) throw new Error('lost prepare response')
      return response(config, order)
    }
    assert.ok(url.endsWith(`/${id}/confirm`))
    confirms++
    assert.equal(recovery.confirm_attempted, true)
    assert.equal(recovery.order_id, id)
    assert.equal(recovery.frozen?.amount_minor, '1234')
    order = { ...order, confirmed_at_unix: '1700000000' }
    throw new Error('lost confirm response')
  }
  const render = async () =>
    act(async () => {
      root.render(
        <I18nextProvider i18n={i18n}>
          <NativeWalletSubscriptions />
        </I18nextProvider>
      )
      await flush()
      await flush()
    })
  const remount = async () => {
    await act(async () => root.unmount())
    root = createRoot(container)
    await render()
  }
  try {
    await render()
    assert.equal(prepares, 0)
    await click('Preview purchase · EPay')
    assert.equal(prepares, 1)
    assert.equal(confirms, 0)
    assert.equal(
      button('Preview purchase').disabled,
      true,
      'balance purchase is blocked too'
    )
    await remount()
    assert.equal(prepares, 1, 'mount must not prepare')
    await click('Retry saved request')
    assert.equal(prepares, 2)
    assert.match(document.body.textContent ?? '', /冻结套餐/)
    assert.match(document.body.textContent ?? '', /CNY 12.34/)
    await click('Confirm')
    assert.equal(confirms, 1)
    assert.equal(posts, 0)
    await remount()
    assert.equal(confirms, 1, 'mount only reads')
    assert.equal(
      button('Continue to payment').disabled,
      true,
      'lost checkout requires read-only recovery'
    )
    assert.ok(
      dom.sessionStorage.getItem(area),
      'unknown confirmation retains original recovery'
    )
    await act(async () => {
      login(53, 'other-account')
      await flush()
      await flush()
    })
    assert.ok(!document.body.textContent?.includes(id))
    assert.equal(confirms, 1)
    assert.ok(
      paths.every((path) => !path.includes('/api/subscription/epay/pay'))
    )
  } finally {
    await act(async () => root.unmount())
    container.remove()
    api.defaults.adapter = originalAdapter
    HTMLFormElement.prototype.submit = originalSubmit
  }
})

test('raw protojson permits review; cancellation recovery stays on original order and broken storage blocks both payment paths', async () => {
  login()
  dom.sessionStorage.clear()
  const original = api.defaults.adapter
  const container = document.body.appendChild(document.createElement('div'))
  let root = createRoot(container)
  let order = rawOrder()
  let confirms = 0,
    cancellations = 0
  api.defaults.adapter = async (config) => {
    const url = config.url ?? ''
    if (config.method === 'get') {
      if (url.endsWith('/epay-prices')) {
        return response(config, {
          prices: [
            {
              plan_id: 4,
              plan_revision: 1,
              channel_id: 9,
              currency: 'CNY',
              amount_minor: '1234',
            },
          ],
        })
      }
      if (url.endsWith('/plans')) {
        return response(config, {
          contract: 'core.subscriptions.v1',
          plans: [plan],
        })
      }
      if (url.endsWith('/purchases')) {
        return response(config, {
          contract: 'core.subscriptions.v1',
          purchases: [],
        })
      }
      return response(config, order)
    }
    if (url.endsWith('/confirm')) confirms++
    if (url.endsWith('/cancel')) {
      cancellations++
      assert.equal(url, `/api/user/payments/subscriptions/orders/${id}/cancel`)
      const raw = dom.sessionStorage.getItem(area)
      assert.ok(raw)
      const saved = epayPendingSchema.parse(JSON.parse(raw))
      assert.equal(saved.cancel_attempted, true)
      assert.equal(saved.confirm_attempted, false)
      order = { ...order, canceled_at_unix: '1700000000' }
      if (cancellations === 1) throw new Error('lost cancellation response')
    }
    return response(config, order)
  }
  const render = async () =>
    act(async () => {
      root.render(
        <I18nextProvider i18n={i18n}>
          <NativeWalletSubscriptions />
        </I18nextProvider>
      )
      await flush()
      await flush()
    })
  try {
    await render()
    await click('Preview purchase · EPay')
    assert.equal(button('Confirm').disabled, false)
    assert.match(document.body.textContent ?? '', /CNY 12.34/)
    await click('Cancel order')
    assert.equal(confirms, 0)
    assert.equal(
      button('Confirm').disabled,
      true,
      'unknown cancellation cannot permit confirmation'
    )
    await act(async () => root.unmount())
    root = createRoot(container)
    await render()
    assert.equal(cancellations, 1, 'reload only reads the canceled order')
    await click('Close')
    assert.equal(dom.sessionStorage.getItem(area), null)
    assert.equal(button('Preview purchase').disabled, false)
    await act(async () => root.unmount())
    root = createRoot(container)
    dom.sessionStorage.setItem(area, '{broken')
    await render()
    assert.match(
      document.body.textContent ?? '',
      /Recovery storage is unavailable/
    )
    assert.equal(button('Preview purchase · EPay').disabled, true)
    assert.equal(button('Preview purchase').disabled, true)
  } finally {
    await act(async () => root.unmount())
    container.remove()
    api.defaults.adapter = original
    dom.sessionStorage.clear()
  }
})

test('current protojson success path persists checkout before GET navigation and reads the original grant after return', async () => {
  login()
  dom.sessionStorage.clear()
  const original = api.defaults.adapter
  const originalAssign = window.location.assign
  const container = document.body.appendChild(document.createElement('div'))
  let root = createRoot(container)
  let order = rawOrder()
  let confirmations = 0,
    preparations = 0,
    updated = 0
  let assigned = ''
  const getCheckout = {
    ...checkout,
    method: 'GET',
    checkout_url: 'https://pay.synthetic.example/submit?money=12.34&sign=exact',
  }
  window.location.assign = (value) => {
    assigned = String(value)
  }
  api.defaults.adapter = async (config) => {
    const url = config.url ?? ''
    if (config.method === 'get') {
      if (url.endsWith('/plans')) {
        return response(config, {
          contract: 'core.subscriptions.v1',
          plans: [plan],
        })
      }
      if (url.endsWith('/purchases')) {
        return response(config, {
          contract: 'core.subscriptions.v1',
          purchases: [],
        })
      }
      if (url.endsWith('/epay-prices')) {
        return response(config, {
          prices: [
            {
              plan_id: 4,
              plan_revision: 1,
              channel_id: 9,
              currency: 'CNY',
              amount_minor: '1234',
            },
          ],
        })
      }
      assert.ok(url.endsWith(id))
      return response(config, {
        ...order,
        checkout:
          order.confirmed_at_unix && !order.proof_id ? getCheckout : null,
      })
    }
    if (url.endsWith('/orders')) {
      preparations++
      return response(config, order)
    }
    assert.ok(url.endsWith(`/${id}/confirm`))
    confirmations++
    order = { ...order, confirmed_at_unix: '1700000000' }
    return response(config, getCheckout)
  }
  const render = async () =>
    act(async () => {
      root.render(
        <I18nextProvider i18n={i18n}>
          <NativeWalletSubscriptions onUpdated={() => updated++} />
        </I18nextProvider>
      )
      await flush()
      await flush()
    })
  const remount = async () => {
    await act(async () => root.unmount())
    root = createRoot(container)
    await render()
  }
  try {
    await render()
    await click('Preview purchase · EPay')
    assert.equal(confirmations, 0)
    await click('Confirm')
    assert.equal(confirmations, 1)
    assert.equal(assigned, '')
    const raw = dom.sessionStorage.getItem(area)
    assert.ok(raw)
    assert.equal(
      epayPendingSchema.parse(JSON.parse(raw)).checkout?.checkout_url,
      getCheckout.checkout_url
    )
    await remount()
    assert.equal(confirmations, 1)
    await click('Continue to payment')
    assert.equal(assigned, getCheckout.checkout_url)
    assert.ok(dom.sessionStorage.getItem(area))
    order = {
      ...order,
      proof_id: 'synthetic-proof',
      subscription_id: '900719925474099' + '3',
      granted_at_unix: '1700000001',
      ends_at_unix: '1702592001',
    }
    await remount()
    assert.match(
      document.body.textContent ?? '',
      /Subscription purchased successfully/
    )
    assert.ok(updated > 0)
    assert.equal(confirmations, 1)
    assert.equal(preparations, 1)
    await click('Close')
    assert.equal(dom.sessionStorage.getItem(area), null)
    assert.equal(button('Preview purchase').disabled, false)
  } finally {
    await act(async () => root.unmount())
    container.remove()
    api.defaults.adapter = original
    window.location.assign = originalAssign
  }
})

test('lost confirm ACK recovers current order.checkout using GET; user continuation rereads checkout and never confirms or purchases twice', async () => {
  login()
  dom.sessionStorage.clear()
  const original = api.defaults.adapter
  const originalSubmit = HTMLFormElement.prototype.submit
  const container = document.body.appendChild(document.createElement('div'))
  let root = createRoot(container)
  let order = { ...rawOrder(), currency: 'LDC', amount_minor: '257' }
  let returnedCheckout = {
    ...checkout,
    checkout_url:
      'https://pay.synthetic.example/submit?order=original&sign=first',
  }
  let prepares = 0,
    confirms = 0
  const submitted: string[][] = []
  HTMLFormElement.prototype.submit = function () {
    assert.equal(this.action, 'https://pay.synthetic.example/submit')
    assert.equal(this.method.toUpperCase(), 'POST')
    submitted.push(
      Array.from(this.querySelectorAll('input')).map(
        (input) => `${input.name}=${input.value}`
      )
    )
  }
  api.defaults.adapter = async (config) => {
    const url = config.url ?? ''
    if (config.method === 'get') {
      if (url.endsWith('/plans')) {
        return response(config, {
          contract: 'core.subscriptions.v1',
          plans: [plan],
        })
      }
      if (url.endsWith('/purchases')) {
        return response(config, {
          contract: 'core.subscriptions.v1',
          purchases: [],
        })
      }
      if (url.endsWith('/epay-prices')) {
        return response(config, {
          prices: [
            {
              plan_id: 4,
              plan_revision: 1,
              channel_id: 9,
              currency: 'LDC',
              amount_minor: '257',
            },
          ],
        })
      }
      assert.equal(url, `/api/user/payments/subscriptions/orders/${id}`)
      return response(config, {
        ...order,
        checkout:
          order.confirmed_at_unix && !order.proof_id ? returnedCheckout : null,
      })
    }
    if (url.endsWith('/orders')) {
      prepares++
      return response(config, { ...order, checkout: null })
    }
    assert.equal(url, `/api/user/payments/subscriptions/orders/${id}/confirm`)
    confirms++
    order = { ...order, confirmed_at_unix: '1700000000' }
    throw new Error('confirm committed but ACK lost')
  }
  const render = async () =>
    act(async () => {
      root.render(
        <I18nextProvider i18n={i18n}>
          <NativeWalletSubscriptions />
        </I18nextProvider>
      )
      await flush()
      await flush()
    })
  try {
    await render()
    await click('Preview purchase · EPay')
    await click('Confirm')
    assert.equal(confirms, 1)
    assert.equal(submitted.length, 0)
    const raw = dom.sessionStorage.getItem(area)
    assert.ok(raw)
    assert.equal(
      epayPendingSchema.parse(JSON.parse(raw)).checkout,
      undefined,
      'ACK loss left no cached checkout'
    )
    await act(async () => root.unmount())
    root = createRoot(container)
    await render()
    assert.equal(confirms, 1)
    assert.equal(prepares, 1)
    assert.equal(button('Continue to payment').disabled, false)
    assert.equal(submitted.length, 0, 'GET recovery never auto-navigates')
    returnedCheckout = {
      ...returnedCheckout,
      checkout_url:
        'https://pay.synthetic.example/submit?order=original&sign=current',
    }
    await click('Continue to payment')
    assert.deepEqual(submitted, [['order=original', 'sign=current']])
    assert.equal(confirms, 1)
    assert.equal(prepares, 1)
    // A later GET with no checkout is authoritative; saved URLs cannot bypass it.
    order = { ...order, proof_id: 'proof-awaiting-grant' }
    await act(async () => root.unmount())
    root = createRoot(container)
    await render()
    assert.ok(
      !Array.from(document.querySelectorAll('button')).some(
        (value) => value.textContent?.trim() === 'Continue to payment'
      )
    )
    assert.equal(submitted.length, 1)
    assert.ok(dom.sessionStorage.getItem(area))
  } finally {
    await act(async () => root.unmount())
    container.remove()
    api.defaults.adapter = original
    HTMLFormElement.prototype.submit = originalSubmit
  }
})
