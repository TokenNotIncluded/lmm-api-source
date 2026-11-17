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
const { CheckinSettingsSection } = await import('./checkin-settings-section')
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
          <CheckinSettingsSection />
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
  assert.ok(check(), 'Check-in settings did not reach the expected state')
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

function configuration() {
  return {
    revision: 7,
    enabled: true,
    min_quota: '10',
    max_quota: '20',
    writable: true,
    module_enabled: false,
    funding_available: false,
    funding_limit: null,
  }
}
test('owner saves exact credits through native settings and reads the persisted revision', async () => {
  let current = configuration()
  const { container, requests } = await mount((config) => {
    assert.equal(config.url, '/api/option/checkin')
    if (config.method === 'put') {
      const saved = body(config)
      assert.equal(saved.revision, 7)
      assert.equal(saved.min_quota, '9007199254740993')
      assert.equal(saved.max_quota, '9007199254740995')
      assert.match(saved.operation_key, /^[a-f0-9-]{36}$/)
      current = { ...current, ...saved, revision: 8 }
    }
    return success(current)
  })
  await until(() => Boolean(container.querySelector('form')))
  assert.match(container.textContent ?? '', /Core funding is unavailable/)
  assert.match(container.textContent ?? '', /The checkins module is disabled/)
  await edit('checkin-maximum', '9007199254740995')
  await edit('checkin-minimum', '9007199254740993')
  await click('Save check-in settings')
  await until(() =>
    [...container.querySelectorAll('button')].some(
      (b) => b.textContent === 'Refresh'
    )
  )
  await click('Refresh')
  await until(() => requests.filter((r) => r.method === 'get').length === 2)
  assert.equal(
    (document.getElementById('checkin-minimum') as HTMLInputElement).value,
    current.min_quota
  )
  assert.equal(requests.filter((r) => r.method === 'put').length, 1)
})

test('an unknown save outcome preserves edits and retries the same operation', async () => {
  const writes: ReturnType<typeof body>[] = []
  let failures = 1
  const { container } = await mount((config) => {
    if (config.method === 'get') return success(configuration())
    writes.push(body(config))
    if (failures-- > 0) rejected(503)
    return success({ ...configuration(), ...body(config), revision: 8 })
  })
  await until(() => Boolean(container.querySelector('form')))
  await edit('checkin-maximum', '30')
  await click('Save check-in settings')
  await until(() =>
    (container.textContent ?? '').includes('Your changes are retained')
  )
  assert.equal(
    (document.getElementById('checkin-maximum') as HTMLInputElement).value,
    '30'
  )
  await click('Save check-in settings')
  await until(() => writes.length === 2)
  assert.deepEqual(writes[1], writes[0])
})

test('a revision conflict preserves the draft until an explicit reload', async () => {
  let current = configuration()
  const { container } = await mount((config) => {
    if (config.method === 'put') {
      current = { ...current, revision: 8, max_quota: '40' }
      rejected(409)
    }
    return success(current)
  })
  await until(() => Boolean(container.querySelector('form')))
  await edit('checkin-maximum', '30')
  await click('Save check-in settings')
  await until(() =>
    (container.textContent ?? '').includes('Settings changed on the server')
  )
  assert.equal(
    (document.getElementById('checkin-maximum') as HTMLInputElement).value,
    '30'
  )
  await click('Discard changes and reload')
  await until(
    () =>
      (document.getElementById('checkin-maximum') as HTMLInputElement).value ===
      '40'
  )
})

test('invalid credits and amounts over the Core ceiling cannot be submitted', async () => {
  const { container, requests } = await mount(() =>
    success({
      ...configuration(),
      funding_available: true,
      funding_limit: '50',
    })
  )
  await until(() => Boolean(container.querySelector('form')))
  for (const value of ['abc', '', '1.5', '-1', '51', '9223372036854775808']) {
    await edit('checkin-maximum', value)
    assert.ok(
      container.querySelector<HTMLButtonElement>('button[type="submit"]')
        ?.disabled
    )
    assert.ok(container.querySelector('[role="alert"]'))
  }
  assert.equal(requests.filter((r) => r.method === 'put').length, 0)
})

test('an ordinary administrator cannot load owner reward settings', async () => {
  const { container, requests } = await mount(
    () => success(configuration()),
    10
  )
  assert.match(container.textContent ?? '', /Only the platform owner/)
  assert.equal(requests.length, 0)
})
