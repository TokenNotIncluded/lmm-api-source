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
const { NativeEPaySettings } = await import('./native-epay-settings')
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
    contract: 'core.epay-configuration.v1',
    revision: 'a'.repeat(64),
    bindings: [
      {
        channel_id: 55,
        merchant_id: '123',
        environment: 'live',
        currency: 'LDC',
        payment_type: 'epay',
        endpoint: 'https://credit.linux.do/epay',
        notify_url: 'https://example.com/callbacks/55',
        return_url: 'https://example.com/wallet',
        key_configured: true,
        enabled: false,
        editable: true,
        credits_per_minor: '500',
        max_amount_minor: '9000000000000',
        min_amount_minor: '10000',
        adapter_configured: true,
        adapter_matches: true,
        wallet_enabled: false,
        subscription_enabled: false,
      },
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
          <NativeEPaySettings />
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

test('EPay owner saves one Core revision and verifies the effective Go projection without exposing the key', async () => {
  let current = configuration()
  const { container, client, requests } = await mount((request) => {
    if (request.method === 'get') {
      assert.equal(request.url, '/api/option/epay/config')
      return success({
        ...current,
        bindings: current.bindings.map((binding) => ({
          ...binding,
          secret_key: 'must-not-cache',
          key_file: '/private/key',
        })),
      })
    }
    assert.equal(request.method, 'put')
    assert.equal(request.url, '/core/v1/payment-gateways/epay/55')
    assert.deepEqual(body(request), {
      expected_revision: 'a'.repeat(64),
      endpoint: 'https://credit.linux.do/epay',
      notify_url: 'https://example.com/callbacks/55',
      return_url: 'https://example.com/saved',
      secret_key: 'synthetic-replacement-signing-key',
    })
    current = {
      ...current,
      revision: 'b'.repeat(64),
      bindings: [
        { ...current.bindings[0], return_url: body(request).return_url },
      ],
    }
    return success(current)
  })
  await until(() => Boolean(container.querySelector('form')))
  assert.match(container.textContent ?? '', /500 CREDIT per 0.01 LDC/)
  assert.match(
    container.textContent ?? '',
    /Legacy amount discounts require a separate pricing review/
  )
  assert.match(container.textContent ?? '', /Minimum wallet top-up/)
  assert.match(container.textContent ?? '', /100\.00 LDC/)
  assert.equal(document.getElementById('epay-min_amount_minor'), null)
  await edit('epay-return_url', 'https://example.com/saved')
  await edit('epay-secret-key', 'synthetic-replacement-signing-key')
  const form = container.querySelector('form')
  assert.ok(form)
  await act(async () => {
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
  })
  await until(
    () =>
      container.textContent?.includes('EPay settings saved and verified') ===
      true
  )
  assert.equal(requests.filter((request) => request.method === 'put').length, 1)
  assert.equal(requests.filter((request) => request.method === 'get').length, 2)
  assert.equal(
    (document.getElementById('epay-secret-key') as HTMLInputElement).value,
    ''
  )
  for (const request of requests) {
    assert.deepEqual(request.authScope, {
      userId: 501,
      sessionId: 'payment-session',
    })
    assert.ok(request.signal)
    assert.equal(request.skipAuthRefresh, true)
  }
  const cached = JSON.stringify(
    client.getQueryData(['native-epay-configuration', 501, 'payment-session'])
  )
  for (const secret of [
    'synthetic-replacement-signing-key',
    'must-not-cache',
    '/private/key',
  ]) {
    assert.ok(!cached.includes(secret))
  }
})

test('stale EPay configuration never replays the write and requires an explicit fresh read', async () => {
  let current = configuration()
  const { container, requests } = await mount((request) => {
    if (request.method === 'get') return success(current)
    current = {
      ...current,
      revision: 'b'.repeat(64),
      bindings: [
        {
          ...current.bindings[0],
          return_url: 'https://example.com/other-owner',
        },
      ],
    }
    return rejected(409)
  })
  await until(() => Boolean(container.querySelector('form')))
  await edit('epay-return_url', 'https://example.com/my-draft')
  await edit('epay-secret-key', 'stale-secret')
  await click('Save EPay settings')
  await until(
    () =>
      container.textContent?.includes('Reload before editing again') === true
  )
  assert.equal(requests.length, 2)
  assert.ok(
    !container.textContent?.includes('EPay settings saved and verified')
  )
  await click('Reload configuration')
  await until(
    () =>
      (document.getElementById('epay-return_url') as HTMLInputElement).value ===
      'https://example.com/other-owner'
  )
  assert.equal(
    (document.getElementById('epay-secret-key') as HTMLInputElement).value,
    ''
  )
  assert.equal(requests.filter((request) => request.method === 'put').length, 1)
  assert.equal(requests.filter((request) => request.method === 'get').length, 2)
})

test('uncertain EPay persistence reads back once and keeps durability uncertainty visible', async () => {
  let current = configuration()
  const { container, requests } = await mount((request) => {
    if (request.method === 'get') return success(current)
    current = {
      ...current,
      revision: 'b'.repeat(64),
      bindings: [
        { ...current.bindings[0], return_url: body(request).return_url },
      ],
    }
    return rejected(503, 'PAYMENT_CONFIGURATION_DURABILITY_UNKNOWN')
  })
  await until(() => Boolean(container.querySelector('form')))
  await edit('epay-return_url', 'https://example.com/uncertain')
  await click('Save EPay settings')
  await until(
    () =>
      container.textContent?.includes('crash durability is unknown') === true
  )
  assert.equal(requests.filter((request) => request.method === 'put').length, 1)
  assert.equal(requests.filter((request) => request.method === 'get').length, 2)
  assert.ok(
    !container.textContent?.includes('EPay settings saved and verified')
  )
  assert.equal(
    (document.getElementById('epay-return_url') as HTMLInputElement).value,
    'https://example.com/uncertain'
  )
})

test('locked EPay history is displayed without permitting a configuration write', async () => {
  const current = configuration()
  current.bindings[0].editable = false
  const { container, requests } = await mount((request) => {
    assert.equal(request.method, 'get')
    return success(current)
  })
  await until(() => Boolean(container.querySelector('form')))
  assert.equal(
    (document.getElementById('epay-return_url') as HTMLInputElement).disabled,
    true
  )
  assert.equal(
    (document.getElementById('epay-secret-key') as HTMLInputElement).disabled,
    true
  )
  await act(async () =>
    container
      .querySelector('form')
      ?.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
  )
  assert.equal(requests.length, 1)
})

test('ordinary administrators cannot inspect EPay configuration', async () => {
  const { container, requests } = await mount(
    () => assert.fail('owner API called'),
    10
  )
  assert.match(container.textContent ?? '', /Owner access required/)
  assert.equal(requests.length, 0)
})

test('changing sessions aborts an EPay save and discards its late acknowledgement', async () => {
  let complete!: (value: unknown) => void
  const pending = new Promise((resolve) => {
    complete = resolve
  })
  const { container, requests } = await mount((request) =>
    request.method === 'get' ? success(configuration()) : pending
  )
  await until(() => Boolean(container.querySelector('form')))
  await edit('epay-return_url', 'https://example.com/late')
  await edit('epay-secret-key', 'old-session-key')
  await click('Save EPay settings')
  const write = requests.find((request) => request.method === 'put')
  assert.ok(write)
  await act(async () => signedIn(100, 'replacement-session'))
  await until(() => Boolean(container.querySelector('form')))
  assert.equal(write.signal?.aborted, true)
  assert.equal(
    (document.getElementById('epay-secret-key') as HTMLInputElement).value,
    ''
  )
  await act(async () => complete(success({ revision: 'b'.repeat(64) })))
  assert.ok(
    !container.textContent?.includes('EPay settings saved and verified')
  )
  assert.equal(requests.filter((request) => request.method === 'get').length, 2)
})
