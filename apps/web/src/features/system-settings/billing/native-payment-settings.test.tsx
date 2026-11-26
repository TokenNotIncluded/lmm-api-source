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
const { NativePaymentGateways } = await import('./native-payment-gateways')
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
  dom.happyDOM.setURL('http://localhost/system-settings/billing/payment')
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
  role = 100,
  Component = NativePaymentSettings
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
          <Component />
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
      container.textContent?.includes('Pancake settings unavailable') === true
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

function bindingsConfiguration() {
  const { contract, ...legacy } = configuration()
  return {
    contract,
    bindings: [
      {
        ...legacy,
        channel_id: 51,
        merchant_id: 'MER_live',
        store_id: 'STO_live',
        product_id: 'PROD_live',
        environment: 'live',
        return_url: 'https://example.com/live',
        channels: [
          {
            id: 51,
            currency: 'USD',
            enabled: false,
            subscription_enabled: false,
          },
          {
            id: 53,
            currency: 'CNY',
            enabled: false,
            subscription_enabled: false,
          },
        ],
      },
      {
        ...legacy,
        channel_id: 52,
        merchant_id: 'MER_test',
        store_id: 'STO_test',
        product_id: 'PROD_test',
        channels: [
          {
            id: 52,
            currency: 'USD',
            enabled: false,
            subscription_enabled: false,
          },
        ],
      },
    ],
  }
}

async function chooseBinding(id: number) {
  const select = document.getElementById(
    'native-payment-binding'
  ) as HTMLSelectElement | null
  assert.ok(select)
  assert.equal(select.disabled, false)
  await act(async () => {
    select.value = String(id)
    select.dispatchEvent(new Event('change', { bubbles: true }))
  })
}

test('multiple disabled payment bindings can be selected without admitting payments or mixing provider actions', async () => {
  const { container, requests } = await mount((config) => {
    if (config.method === 'get') return success(bindingsConfiguration())
    assert.equal(body(config).channel_id, 52)
    assert.equal(body(config).merchant_id, 'MER_test')
    assert.equal(body(config).store_id, 'STO_test')
    if (config.url?.endsWith('/catalog')) {
      return success({
        stores: [
          {
            id: 'STO_catalog',
            name: 'Selected test shop',
            status: 'active',
            prodEnabled: false,
            onetimeProducts: [],
            subscriptionProducts: [],
          },
        ],
      })
    }
    assert.equal(config.url, '/api/option/waffo-pancake/pair')
    return success({
      store_id: 'STO_new',
      store_name: 'New test store',
      product_id: 'PROD_new',
      product_name: 'New test product',
    })
  })
  await until(() => Boolean(container.querySelector('form')))
  const selector = document.getElementById(
    'native-payment-binding'
  ) as HTMLSelectElement
  assert.match(
    selector.textContent ?? '',
    /Production · USD \/ CNY · Channel #51/
  )
  assert.match(
    selector.textContent ?? '',
    /Test environment · USD · Channel #52/
  )
  assert.match(container.querySelector('dl')?.textContent ?? '', /MER_live/)
  assert.match(
    container.querySelector('dl[aria-label="USD · Channel #51"]')
      ?.textContent ?? '',
    /Disabled/
  )
  await chooseBinding(52)
  assert.equal(requests.length, 1)
  assert.match(container.querySelector('dl')?.textContent ?? '', /MER_test/)
  assert.ok(!container.querySelector('dl')?.textContent?.includes('MER_live'))
  await click('Load provider catalog')
  await until(
    () => container.textContent?.includes('Selected test shop') === true
  )
  await click('Create store and product')
  assert.equal(requests.length, 2)
  await click('Create provider objects')
  await until(
    () =>
      container.textContent?.includes(
        'Provider objects created; activation required'
      ) === true
  )
  assert.equal(requests.length, 3)
  assert.ok(!requests.some((request) => request.url?.endsWith('/save')))
  await edit('native-payment-private-key', 'selected-draft-key')
  assert.equal(selector.disabled, true)
  assert.match(
    container.textContent ?? '',
    /Save or discard changes before switching/
  )
  await click('Discard changes')
  assert.equal(selector.disabled, false)
  await chooseBinding(51)
  assert.match(container.querySelector('dl')?.textContent ?? '', /MER_live/)
  assert.ok(!container.textContent?.includes('Selected test shop'))
  assert.ok(!container.textContent?.includes('STO_new'))
  assert.equal(
    (
      document.getElementById(
        'native-payment-private-key'
      ) as HTMLTextAreaElement
    ).value,
    ''
  )
  assert.equal(requests.length, 3)
})

for (const uncertain of [false, true]) {
  test(`selected payment binding survives ${
    uncertain ? 'uncertain' : 'successful'
  } save readback without changing another binding`, async () => {
    let current = bindingsConfiguration()
    const originalLive = structuredClone(current.bindings[0])
    let releaseSave!: () => void
    const saveGate = new Promise<void>((resolve) => {
      releaseSave = resolve
    })
    const { container, client, requests } = await mount(async (config) => {
      if (config.method === 'get') {
        return success({
          ...current,
          bindings: current.bindings.map((binding) => ({
            ...binding,
            private_key: 'must-not-cache-binding-secret',
            private_key_file: '/synthetic/private/key.pem',
          })),
        })
      }
      assert.equal(config.url, '/api/option/waffo-pancake/save')
      const values = body(config)
      assert.deepEqual(values, {
        channel_id: 52,
        merchant_id: 'MER_test',
        store_id: 'STO_test',
        product_id: 'PROD_test',
        return_url: 'https://example.com/test-saved',
        private_key: 'synthetic-selected-key',
      })
      await saveGate
      current = {
        ...current,
        bindings: current.bindings.map((binding) =>
          binding.channel_id === values.channel_id
            ? { ...binding, return_url: values.return_url }
            : binding
        ),
      }
      if (uncertain) {
        return rejected(503, 'PAYMENT_CONFIGURATION_DURABILITY_UNKNOWN')
      }
      return success({
        store_id: values.store_id,
        product_id: values.product_id,
      })
    })
    await until(() => Boolean(container.querySelector('form')))
    await chooseBinding(52)
    await edit('native-payment-return-url', 'https://example.com/test-saved')
    await edit('native-payment-private-key', 'synthetic-selected-key')
    await click('Save payment settings')
    const selector = document.getElementById(
      'native-payment-binding'
    ) as HTMLSelectElement
    assert.equal(selector.disabled, true)
    await act(async () => releaseSave())
    await until(
      () =>
        container.textContent?.includes(
          uncertain
            ? 'crash durability is unknown'
            : 'Payment settings saved and read back'
        ) === true
    )
    assert.equal(selector.value, '52')
    assert.equal(selector.disabled, false)
    assert.equal(
      (document.getElementById('native-payment-return-url') as HTMLInputElement)
        .value,
      'https://example.com/test-saved'
    )
    assert.equal(
      (
        document.getElementById(
          'native-payment-private-key'
        ) as HTMLTextAreaElement
      ).value,
      ''
    )
    assert.deepEqual(current.bindings[0], originalLive)
    assert.equal(
      requests.filter((request) => request.method === 'post').length,
      1
    )
    assert.equal(
      requests.filter((request) => request.method === 'get').length,
      2
    )
    const cached = JSON.stringify(
      client.getQueryData([
        'native-payment-configuration',
        501,
        'payment-session',
      ])
    )
    for (const secret of [
      'synthetic-selected-key',
      'must-not-cache-binding-secret',
      '/synthetic/private/key.pem',
    ]) {
      assert.ok(!cached.includes(secret))
    }
    if (uncertain) {
      assert.ok(
        !container.textContent?.includes('Payment settings saved and read back')
      )
    }
  })
}

test('readback keeps the selected currency when equivalent bindings regroup after save', async () => {
  const config = bindingsConfiguration()
  const live = config.bindings[0]
  config.bindings = [
    { ...live, channels: [live.channels[0]] },
    {
      ...live,
      channel_id: 53,
      return_url: 'https://example.com/cny',
      channels: [live.channels[1]],
    },
  ]
  let current = config
  const { container, requests } = await mount((request) => {
    if (request.method === 'get') return success(current)
    assert.equal(body(request).channel_id, 53)
    current = { ...current, bindings: [live] }
    return success({ store_id: live.store_id, product_id: live.product_id })
  })
  await until(() => Boolean(container.querySelector('form')))
  await chooseBinding(53)
  await edit('native-payment-return-url', live.return_url)
  await click('Save payment settings')
  await until(
    () =>
      container.textContent?.includes(
        'Payment settings saved and read back'
      ) === true
  )
  assert.equal(document.getElementById('native-payment-binding'), null)
  assert.match(
    container.querySelector('dl[aria-label="USD · Channel #51"]')
      ?.textContent ?? '',
    /USD #51/
  )
  assert.match(
    container.querySelector('dl[aria-label="CNY · Channel #53"]')
      ?.textContent ?? '',
    /CNY #53/
  )
  assert.equal(
    requests.filter((request) => request.method === 'post').length,
    1
  )
})

test('payment callbacks follow the selected environment and current site without saving automatically', async () => {
  dom.happyDOM.setURL(
    'https://native.example.test/system-settings/billing/payment'
  )
  const { container, requests } = await mount(() =>
    success(bindingsConfiguration())
  )
  await until(() => Boolean(container.querySelector('form')))
  const webhook = () =>
    (document.getElementById('native-payment-webhook') as HTMLInputElement)
      .value
  assert.equal(
    webhook(),
    'https://native.example.test/api/waffo-pancake/webhook/prod'
  )
  await click('Copy webhook URL')
  assert.equal(await navigator.clipboard.readText(), webhook())
  await chooseBinding(52)
  assert.equal(
    webhook(),
    'https://native.example.test/api/waffo-pancake/webhook/test'
  )
  await click('Use this site’s wallet URL')
  assert.equal(
    (document.getElementById('native-payment-return-url') as HTMLInputElement)
      .value,
    'https://native.example.test/wallet'
  )
  assert.equal(requests.length, 1)
  assert.ok(requests.every((request) => request.method === 'get'))
})

for (const callbacksEnabled of [undefined, false, true]) {
  test(`payment callback admission remains explicit when the host reports ${callbacksEnabled}`, async () => {
    const { container } = await mount(() =>
      success({
        ...configuration(),
        ...(callbacksEnabled === undefined
          ? {}
          : { callbacks_enabled: callbacksEnabled }),
      })
    )
    await until(() => Boolean(container.querySelector('form')))
    const section = container.querySelector(
      'section[aria-labelledby="payment-webhook-heading"]'
    )
    assert.ok(section)
    const callbackState = [...section.querySelectorAll('dt')].find(
      (label) => label.textContent === 'Callback processing'
    )?.nextElementSibling?.textContent
    assert.equal(
      callbackState,
      callbacksEnabled === undefined
        ? 'Not reported'
        : callbacksEnabled
          ? 'Enabled'
          : 'Disabled'
    )
    assert.equal(
      section.textContent?.includes(
        'Payment notifications are disabled on this server.'
      ),
      callbacksEnabled === false
    )
  })
}

test('Pancake reload keeps unsaved credentials until explicit confirmation and sends no writes', async () => {
  let current = configuration()
  const { container, requests } = await mount((request) => {
    assert.equal(request.method, 'get')
    return success(current)
  })
  await until(() => Boolean(container.querySelector('form')))
  await edit('native-payment-return-url', 'https://example.com/unsaved')
  await edit('native-payment-private-key', 'unsaved-secret')
  current = { ...current, return_url: 'https://example.com/updated' }
  await click('Reload configuration')
  assert.equal(requests.length, 1)
  assert.equal(
    (
      document.getElementById(
        'native-payment-private-key'
      ) as HTMLTextAreaElement
    ).value,
    'unsaved-secret'
  )
  await click('Cancel')
  assert.equal(requests.length, 1)
  await click('Reload configuration')
  await click('Discard changes and reload')
  await until(
    () =>
      (document.getElementById('native-payment-return-url') as HTMLInputElement)
        .value === current.return_url
  )
  assert.equal(requests.length, 2)
  assert.equal(
    (
      document.getElementById(
        'native-payment-private-key'
      ) as HTMLTextAreaElement
    ).value,
    ''
  )
  assert.ok(requests.every((request) => request.method === 'get'))
})

test('gateway overview lists native protocols and wallet availability independently of the Pancake editor', async () => {
  const { container, requests } = await mount(
    (config) => {
      if (config.url === '/api/user/payments/channels') {
        return success({
          available: true,
          channels: [
            {
              id: '37',
              provider: 'stripe',
              currency: 'USD',
              environment: 'test',
            },
          ],
        })
      }
      return rejected(404, 'PAYMENT_PANCAKE_NOT_CONFIGURED')
    },
    100,
    NativePaymentGateways
  )
  await until(() => container.textContent?.includes('#37') === true)
  const methods = container.querySelector('dl')!
  for (const name of [
    'Epay Alipay',
    'Epay WeChat Pay',
    'LINUX DO Credit',
    'Stripe',
    'Creem',
    'Waffo Global',
    'Waffo Pancake',
    'Coinbase Business Checkout',
  ]) {
    assert.ok(
      [...methods.querySelectorAll('dt')].some(
        (term) => term.textContent === name
      )
    )
  }
  assert.match(methods.textContent ?? '', /Stripe#37 · USD · Test payment/)
  assert.match(methods.textContent ?? '', /CreemNot offered to this wallet/)
  assert.deepEqual(
    requests.map((request) => request.url),
    ['/api/user/payments/channels']
  )
  await click('Waffo Pancake')
  await until(
    () =>
      container.textContent?.includes(
        'No Waffo Pancake channel is configured.'
      ) === true
  )
  assert.equal(container.querySelector('form'), null)
  await click('Wallet payment methods')
  await until(
    () => container.querySelector('dl')?.textContent?.includes('#37') === true
  )
  assert.ok(requests.every((request) => request.method === 'get'))
})

test('a failed wallet catalogue leaves the supported methods visible and their availability unknown', async () => {
  const { container } = await mount(
    () => rejected(503),
    100,
    NativePaymentGateways
  )
  await until(
    () =>
      container
        .querySelector('dl')
        ?.textContent?.includes('Availability unknown') === true
  )
  const methods = container.querySelector('dl')!
  assert.match(methods.textContent ?? '', /StripeAvailability unknown/)
  assert.doesNotMatch(methods.textContent ?? '', /Not offered to this wallet/)
})

test('a missing route stays distinct from a verified empty Pancake configuration', async () => {
  const { container } = await mount(() => rejected(404))
  await until(
    () =>
      container.textContent?.includes(
        'The Pancake settings route is missing.'
      ) === true
  )
  assert.doesNotMatch(
    container.textContent ?? '',
    /No Waffo Pancake channel is configured/
  )
})
