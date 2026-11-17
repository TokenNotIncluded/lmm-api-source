/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { Window } from 'happy-dom'

import type { ApiRequestConfig } from '@/lib/api'

const dom = new Window({
  url: 'http://localhost/system-settings/billing/payment',
  settings: { device: { prefersReducedMotion: 'reduce' } },
})
for (const [name, value] of Object.entries({
  window: dom,
  document: dom.document,
  navigator: dom.navigator,
  HTMLElement: dom.HTMLElement,
  Element: dom.Element,
  Node: dom.Node,
  Event: dom.Event,
  MouseEvent: dom.MouseEvent,
  MutationObserver: dom.MutationObserver,
  ResizeObserver: dom.ResizeObserver,
  HTMLInputElement: dom.HTMLInputElement,
  HTMLTextAreaElement: dom.HTMLTextAreaElement,
  HTMLButtonElement: dom.HTMLButtonElement,
  DocumentFragment: dom.DocumentFragment,
  ShadowRoot: dom.ShadowRoot,
  requestAnimationFrame: dom.requestAnimationFrame.bind(dom),
  cancelAnimationFrame: dom.cancelAnimationFrame.bind(dom),
  getComputedStyle: dom.getComputedStyle.bind(dom),
})) {
  Object.defineProperty(globalThis, name, { configurable: true, value })
}
Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
  configurable: true,
  value: true,
})

const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider, notifyManager } =
  await import('@tanstack/react-query')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { NativePaymentSettings } = await import('./native-payment-settings')
notifyManager.setScheduler(queueMicrotask)
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
const previousAuth = useAuthStore.getState().auth
const previousAdapter = api.defaults.adapter
let dispose: (() => Promise<void>) | undefined
afterEach(async () => {
  await dispose?.()
  dispose = undefined
  api.defaults.adapter = previousAdapter
  useAuthStore.setState({ auth: previousAuth })
  document.body.replaceChildren()
})
after(() => dom.close())

function configuration() {
  return {
    contract: 'extensions.pancake-configuration.v1',
    merchant_id: 'MER_fixture',
    store_id: 'STO_fixture',
    product_id: 'PROD_fixture',
    return_url: 'https://example.com/wallet',
    environment: 'test',
    private_key_configured: true,
    channels: [
      { id: 7, currency: 'USD', enabled: true, subscription_enabled: false },
    ],
  }
}
function signedIn(role = 100, sid = 'payment-session') {
  useAuthStore.getState().auth.setBundle({
    access_token: 'synthetic-payment-session',
    token_type: 'Bearer',
    access_expires_at: 2_000_000_000,
    user: { id: 501, role, username: 'synthetic-owner' },
    session: {
      sid,
      current: true,
      login_method: 'password',
      ip: '127.0.0.1',
      user_agent: 'synthetic',
      created_at: 1,
      last_active_at: 1,
      expires_at: 2_000_000_000,
    },
  })
}
const success = (data: unknown) => ({ success: true, data })
function rejected(status: number, code = '', data?: unknown): never {
  throw {
    isAxiosError: true,
    response: { status, data: { success: false, code, data } },
  }
}
async function mount(
  handler: (config: ApiRequestConfig) => unknown | Promise<unknown>,
  role = 100
) {
  signedIn(role)
  const requests: ApiRequestConfig[] = []
  api.defaults.adapter = async (config) => {
    requests.push(config)
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: await handler(config),
    }
  }
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  await act(async () =>
    root.render(
      <QueryClientProvider client={client}>
        <I18nextProvider i18n={i18n}>
          <NativePaymentSettings />
        </I18nextProvider>
      </QueryClientProvider>
    )
  )
  dispose = async () => {
    await act(async () => root.unmount())
    client.clear()
  }
  return { container, client, requests }
}
async function until(check: () => boolean) {
  for (let attempt = 0; attempt < 100 && !check(); attempt++) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 10))
    })
  }
  assert.ok(check(), 'Payment settings did not reach the expected state')
}
async function click(label: string) {
  const button = [
    ...document.querySelectorAll<HTMLButtonElement>('button'),
  ].find((element) => element.textContent === label)
  assert.ok(button, `Missing button: ${label}`)
  await act(async () => button.click())
}
async function edit(id: string, value: string) {
  const input = document.getElementById(id) as
    | HTMLInputElement
    | HTMLTextAreaElement
    | null
  assert.ok(input)
  const prototype =
    input.tagName === 'TEXTAREA'
      ? dom.HTMLTextAreaElement.prototype
      : dom.HTMLInputElement.prototype
  const setter = Object.getOwnPropertyDescriptor(prototype, 'value')?.set
  assert.ok(setter)
  await act(async () => {
    setter.call(input, value)
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
}
function body(config: ApiRequestConfig) {
  return typeof config.data === 'string' ? JSON.parse(config.data) : config.data
}

test('owner edits payment settings, saves once and reads back without caching private keys', async () => {
  let current = configuration()
  const { container, client, requests } = await mount((config) => {
    if (config.method === 'get') {
      return success({ ...current, private_key: 'must-not-be-cached' })
    }
    assert.equal(config.url, '/api/option/waffo-pancake/save')
    const values = body(config)
    assert.deepEqual(values, {
      merchant_id: current.merchant_id,
      store_id: current.store_id,
      product_id: current.product_id,
      return_url: 'https://example.com/paid',
      private_key: 'synthetic-replacement-key',
    })
    current = { ...current, return_url: values.return_url }
    return success({
      store_id: current.store_id,
      product_id: current.product_id,
    })
  })
  await until(() => Boolean(container.querySelector('form')))
  assert.match(container.textContent ?? '', /MER_fixture/)
  assert.equal(
    (
      document.getElementById(
        'native-payment-private-key'
      ) as HTMLTextAreaElement
    ).value,
    ''
  )
  await edit('native-payment-return-url', 'https://example.com/paid')
  await edit('native-payment-private-key', 'synthetic-replacement-key')
  const form = container.querySelector('form')
  assert.ok(form)
  await act(async () => {
    for (let index = 0; index < 2; index++) {
      form.dispatchEvent(
        new Event('submit', { bubbles: true, cancelable: true })
      )
    }
  })
  await until(
    () =>
      container.textContent?.includes(
        'Payment settings saved and read back'
      ) === true
  )
  assert.equal(
    requests.filter((request) => request.method === 'post').length,
    1
  )
  assert.equal(requests.filter((request) => request.method === 'get').length, 2)
  for (const request of requests) {
    assert.deepEqual(request.authScope, {
      userId: 501,
      sessionId: 'payment-session',
    })
    assert.ok(request.signal)
    assert.equal(request.skipAuthRefresh, true)
  }
  assert.equal(
    (
      document.getElementById(
        'native-payment-private-key'
      ) as HTMLTextAreaElement
    ).value,
    ''
  )
  const cached = JSON.stringify(
    client.getQueryData([
      'native-payment-configuration',
      501,
      'payment-session',
    ])
  )
  assert.ok(!cached.includes('synthetic-replacement-key'))
  assert.ok(!cached.includes('must-not-be-cached'))
  assert.ok(!requests.some((request) => request.url === '/api/option/'))
})

test('ordinary administrators cannot read or edit owner payment configuration', async () => {
  const { container, requests } = await mount(
    () => assert.fail('Owner-only API called'),
    10
  )
  assert.match(container.textContent ?? '', /Owner access required/)
  assert.equal(requests.length, 0)
  assert.equal(container.querySelector('form'), null)
})

test('unavailable configuration shows truthful host setup and can retry the real read', async () => {
  let available = false
  const { container, requests } = await mount(() =>
    available ? success(configuration()) : rejected(503)
  )
  await until(
    () =>
      container.textContent?.includes('Payment settings unavailable') === true
  )
  assert.equal(container.querySelector('form'), null)
  assert.match(container.textContent ?? '', /payment_token_file/)
  assert.match(container.textContent ?? '', /database_url_file/)
  assert.ok(!container.textContent?.includes('Payment settings saved'))
  available = true
  await click('Retry')
  await until(() => Boolean(container.querySelector('form')))
  assert.equal(requests.length, 2)
})

test('catalog and confirmed pairing use real scoped actions without activating a new binding', async () => {
  const { container, requests } = await mount((config) => {
    if (config.method === 'get') return success(configuration())
    assert.equal(body(config).private_key, 'probe-key')
    if (config.url?.endsWith('/catalog')) {
      return success({
        stores: [
          {
            id: 'STO_catalog',
            name: 'Provider shop',
            status: 'active',
            prodEnabled: true,
            onetimeProducts: [
              { id: 'PROD_catalog', name: 'Wallet credits', status: 'active' },
            ],
            subscriptionProducts: [],
          },
        ],
      })
    }
    if (config.url?.endsWith('/pair')) {
      return success({
        store_id: 'STO_new',
        store_name: 'New store',
        product_id: 'PROD_new',
        product_name: 'New product',
      })
    }
    assert.fail(`Unexpected write: ${config.url}`)
  })
  await until(() => Boolean(container.querySelector('form')))
  await edit('native-payment-private-key', 'probe-key')
  assert.equal(requests.length, 1)
  await click('Load provider catalog')
  await until(() => container.textContent?.includes('Provider shop') === true)
  assert.match(container.textContent ?? '', /Wallet credits/)
  await click('Create store and product')
  assert.equal(requests.length, 2)
  await click('Create provider objects')
  await until(
    () =>
      container.textContent?.includes(
        'Provider objects created; activation required'
      ) === true
  )
  assert.match(container.textContent ?? '', /STO_new/)
  assert.equal(requests.length, 3)
  assert.ok(!requests.some((request) => request.url?.endsWith('/save')))
  assert.match(container.querySelector('dl')?.textContent ?? '', /STO_fixture/)
})

test('lost save response triggers one readback and never reports an unconfirmed write as saved', async () => {
  let current = configuration()
  const { container, requests } = await mount((config) => {
    if (config.method === 'get') return success(current)
    current = { ...current, return_url: body(config).return_url }
    return rejected(503, 'PAYMENT_CONFIGURATION_DURABILITY_UNKNOWN')
  })
  await until(() => Boolean(container.querySelector('form')))
  await edit('native-payment-return-url', 'https://example.com/uncertain')
  await click('Save payment settings')
  await until(
    () =>
      container.textContent?.includes('crash durability is unknown') === true
  )
  assert.equal(
    requests.filter((request) => request.method === 'post').length,
    1
  )
  assert.equal(requests.filter((request) => request.method === 'get').length, 2)
  assert.ok(
    !container.textContent?.includes('Payment settings saved and read back')
  )
  assert.equal(
    (document.getElementById('native-payment-return-url') as HTMLInputElement)
      .value,
    'https://example.com/uncertain'
  )
})

test('a partial provider pair keeps the created store visible for recovery', async () => {
  const { container, requests } = await mount((config) =>
    config.method === 'get'
      ? success(configuration())
      : rejected(502, 'PAYMENT_PAIR_INCOMPLETE', {
          store_id: 'STO_partial',
          store_name: 'Partial store',
          orphan_store: true,
        })
  )
  await until(() => Boolean(container.querySelector('form')))
  await click('Create store and product')
  await click('Create provider objects')
  await until(
    () => container.textContent?.includes('Product setup incomplete') === true
  )
  assert.match(container.textContent ?? '', /STO_partial/)
  assert.match(
    container.textContent ?? '',
    /Review the returned store before trying again/
  )
  assert.equal(
    requests.filter((request) => request.method === 'post').length,
    1
  )
})

test('session replacement clears drafts, aborts the previous action and discards its late response', async () => {
  let complete!: (value: unknown) => void
  const lateCatalog = new Promise((resolve) => {
    complete = resolve
  })
  const { container, requests } = await mount((config) =>
    config.method === 'get'
      ? success({
          ...configuration(),
          merchant_id:
            config.authScope?.sessionId === 'replacement-session'
              ? 'MER_replacement'
              : 'MER_fixture',
        })
      : lateCatalog
  )
  await until(() => Boolean(container.querySelector('form')))
  await edit('native-payment-private-key', 'old-session-secret')
  await click('Load provider catalog')
  const oldRequest = requests.find((request) => request.method === 'post')
  assert.ok(oldRequest)
  await act(async () => signedIn(100, 'replacement-session'))
  await until(() => container.textContent?.includes('MER_replacement') === true)
  assert.equal(oldRequest.signal?.aborted, true)
  assert.equal(
    (
      document.getElementById(
        'native-payment-private-key'
      ) as HTMLTextAreaElement
    ).value,
    ''
  )
  await act(async () =>
    complete(
      success({
        stores: [
          {
            id: 'STO_old',
            name: 'Old session store',
            status: 'active',
            prodEnabled: true,
            onetimeProducts: [],
            subscriptionProducts: [],
          },
        ],
      })
    )
  )
  assert.ok(!container.textContent?.includes('Old session store'))
  assert.equal(
    requests.filter((request) => request.method === 'post').length,
    1
  )
})

test('server revocation removes configuration and credentials from the mounted workspace', async () => {
  const { container } = await mount((config) =>
    config.method === 'get' ? success(configuration()) : rejected(403)
  )
  await until(() => Boolean(container.querySelector('form')))
  await edit('native-payment-private-key', 'revoked-session-secret')
  await click('Load provider catalog')
  await until(
    () => container.textContent?.includes('Payment access denied') === true
  )
  assert.equal(container.querySelector('textarea'), null)
  assert.ok(!container.textContent?.includes('MER_fixture'))
})
