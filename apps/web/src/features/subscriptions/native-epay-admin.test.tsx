/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, test } from 'node:test'

import type { InternalAxiosRequestConfig } from 'axios'
import { Window } from 'happy-dom'

import type { SaveEPayPrice } from './native-epay-admin-api'

const dom = new Window({
  url: 'https://synthetic.example/subscriptions',
  settings: { device: { prefersReducedMotion: 'reduce' } },
})
dom.document.write('<!doctype html><html><head></head><body></body></html>')
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLInputElement',
  'HTMLSelectElement',
  'HTMLButtonElement',
  'SVGElement',
  'Node',
  'Element',
  'Event',
  'MouseEvent',
  'CustomEvent',
  'MutationObserver',
  'ResizeObserver',
  'DocumentFragment',
  'ShadowRoot',
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
const { NativeSubscriptionAdmin } = await import('./native-admin')
const { saveAdminEPayPrice, listAdminEPayPrices, pendingEPayPriceSchema } =
  await import('./native-epay-admin-api')
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
after(() => dom.close())
const flush = () => new Promise((resolve) => setTimeout(resolve, 20))
const scope = { userId: 52, sessionId: 'synthetic-price-admin' }
const area = 'lmm-commerce:52:synthetic-price-admin:subscription-epay-prices'
const plan = {
  id: 4,
  revision: 1,
  archived: false,
  terms: {
    title: 'Disabled native plan',
    price_units: '100000',
    duration_seconds: 0,
    duration_months: 1,
    reset_period: 'grant',
    reset_seconds: 0,
    limit_units: '1000',
    models: ['*'],
    groups: ['*'],
    max_purchases_per_user: 0,
    enabled: false,
  },
}
const price = {
  plan_id: 4,
  plan_revision: 1,
  channel_id: 9,
  currency: 'CNY',
  amount_minor: '1234',
} satisfies SaveEPayPrice['price']

function login(role = 100, user = 52, sid = scope.sessionId) {
  useAuthStore.getState().auth.setBundle({
    user: {
      id: user,
      role,
      username: 'synthetic-root',
      wallet_unit: 'core_credit',
    },
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
function response(config: InternalAxiosRequestConfig, data: unknown) {
  return { config, data, headers: {}, status: 200, statusText: 'OK' }
}
function button(label: string) {
  const found = Array.from(document.querySelectorAll('button')).find(
    (value) => value.textContent?.trim() === label
  )
  assert.ok(found, `Missing button ${label}`)
  return found
}
async function click(label: string) {
  await act(async () => {
    button(label).click()
    await flush()
    await flush()
  })
}
async function mount() {
  const container = document.body.appendChild(document.createElement('div'))
  let root = createRoot(container)
  async function render() {
    await act(async () => {
      root.render(
        <I18nextProvider i18n={i18n}>
          <NativeSubscriptionAdmin />
        </I18nextProvider>
      )
      await flush()
      await flush()
    })
  }
  await render()
  return {
    render,
    remount: async () => {
      await act(async () => root.unmount())
      root = createRoot(container)
      await render()
    },
    close: async () => {
      await act(async () => root.unmount())
      container.remove()
    },
  }
}
async function select(value: string) {
  const input = document.querySelector('#epay-price-channel')
  assert.ok(input instanceof HTMLSelectElement)
  await act(async () => {
    input.value = value
    input.dispatchEvent(new Event('change', { bubbles: true }))
    await flush()
  })
}
async function amount(value: string) {
  const input = document.querySelector('#epay-price-amount')
  assert.ok(input instanceof HTMLInputElement)
  const setter = Object.getOwnPropertyDescriptor(
    HTMLInputElement.prototype,
    'value'
  )?.set
  assert.ok(setter)
  await act(async () => {
    setter.call(input, value)
    input.dispatchEvent(new Event('input', { bubbles: true }))
    await flush()
  })
}

test('root native administration reads disabled-plan prices, creates only a new channel price and recovers the exact saved operation after ACK loss', async () => {
  login()
  dom.sessionStorage.clear()
  const original = api.defaults.adapter
  const requests: unknown[] = []
  const reads: string[] = []
  let configured: SaveEPayPrice['price'][] = [price]
  let key = ''
  api.defaults.adapter = async (config) => {
    const url = config.url ?? ''
    if (config.method === 'get') {
      reads.push(url)
      if (url.endsWith('/admin/plans')) {
        return response(config, {
          contract: 'core.subscriptions.v1',
          plans: [plan],
        })
      }
      if (url.endsWith('/epay-prices')) {
        assert.equal(url, '/core/v1/subscriptions/admin/plans/4/epay-prices')
        return response(config, { prices: configured })
      }
      assert.equal(url, '/api/user/payments/channels')
      return response(config, {
        success: true,
        data: {
          available: true,
          channels: [
            {
              id: '9',
              provider: 'epay',
              environment: 'test',
              currency: 'CNY',
              payment_type: 'alipay',
            },
            {
              id: '10',
              provider: 'epay',
              environment: 'live',
              currency: 'LDC',
              payment_type: 'epay',
            },
            {
              id: '11',
              provider: 'stripe',
              environment: 'test',
              currency: 'USD',
            },
          ],
        },
      })
    }
    assert.equal(url, '/core/v1/subscriptions/admin/plans/4/epay-prices')
    const raw = dom.sessionStorage.getItem(area)
    assert.ok(raw)
    const saved = pendingEPayPriceSchema.parse(JSON.parse(raw))
    const body = JSON.parse(String(config.data))
    assert.deepEqual(body, saved.request)
    if (!key) key = body.operation_key
    assert.equal(body.operation_key, key)
    assert.deepEqual(body.price, {
      plan_id: 4,
      plan_revision: 1,
      channel_id: 10,
      currency: 'LDC',
      amount_minor: '257',
    })
    requests.push(body)
    configured = [price, body.price]
    if (requests.length === 1) throw new Error('lost price ACK')
    return response(config, body.price)
  }
  const view = await mount()
  try {
    await click('EPay · Price')
    assert.match(document.body.textContent ?? '', /CNY 12.34/)
    assert.match(document.body.textContent ?? '', /Disabled native plan/)
    assert.equal(document.querySelector('option[value="11"]'), null)
    await select('9')
    assert.equal(
      button('Save').disabled,
      true,
      'same revision/channel price is immutable'
    )
    assert.ok(button('Edit Plan · Revision'))
    await select('10')
    await amount('2.57')
    assert.equal(button('Save').disabled, false)
    await click('Save')
    assert.equal(requests.length, 1)
    assert.equal(button('Create Plan').disabled, true)
    assert.match(document.body.textContent ?? '', /Result unknown/)
    await view.remount()
    assert.equal(
      requests.length,
      1,
      'mount reads only; it cannot resubmit a price'
    )
    await click('Retry saved request')
    assert.equal(requests.length, 2)
    assert.deepEqual(requests[0], requests[1])
    assert.equal(dom.sessionStorage.getItem(area), null)
    assert.match(document.body.textContent ?? '', /LDC 2.57/)
    assert.match(document.body.textContent ?? '', /Saved successfully/)
    assert.equal(button('Save').disabled, true)
    await click('Close')
    assert.equal(button('Create Plan').disabled, false)
    assert.ok(
      reads.includes('/core/v1/subscriptions/admin/plans/4/epay-prices')
    )
    assert.ok(reads.every((url) => !url.includes('/api/subscription/epay/pay')))
  } finally {
    await view.close()
    api.defaults.adapter = original
  }
})

test('ordinary administrators and buyers never see price configuration; admin API blocks non-root and role changes in flight', async () => {
  dom.sessionStorage.clear()
  login(10)
  const original = api.defaults.adapter
  let writes = 0
  const reads: string[] = []
  api.defaults.adapter = async (config) => {
    if (config.method !== 'get') writes++
    reads.push(config.url ?? '')
    return response(config, {
      contract: 'core.subscriptions.v1',
      items: [],
      total: 0,
      page: 1,
      page_size: 20,
    })
  }
  const view = await mount()
  const request = { operation_key: 'synthetic-price-key', price }
  try {
    assert.ok(!document.body.textContent?.includes('EPay · Price'))
    assert.ok(
      reads.every(
        (url) =>
          !url.includes('/epay-prices') && !url.includes('/payments/channels')
      )
    )
    await assert.rejects(saveAdminEPayPrice(scope, request), /FORBIDDEN/)
    assert.equal(writes, 0)
    await act(async () => {
      login(1)
      await flush()
    })
    assert.match(document.body.textContent ?? '', /Access Forbidden/)
  } finally {
    await view.close()
    api.defaults.adapter = original
  }
})

test('price API rechecks root authority and current session after responses', async () => {
  const original = api.defaults.adapter
  const request = { operation_key: 'synthetic-price-key', price }
  try {
    login()
    api.defaults.adapter = async (config) => {
      login(10)
      return response(config, price)
    }
    await assert.rejects(saveAdminEPayPrice(scope, request), /FORBIDDEN/)
    login()
    api.defaults.adapter = async (config) => {
      login(100, 53, 'other')
      return response(config, { prices: [price] })
    }
    await assert.rejects(listAdminEPayPrices(scope, 4), /SESSION_CHANGED/)
  } finally {
    api.defaults.adapter = original
  }
})
