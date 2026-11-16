/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
// @ts-expect-error Bun's test module is available only in the test runtime.
import { mock as moduleMock } from 'bun:test'
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { Window } from 'happy-dom'
import type { ReactNode } from 'react'

const dom = new Window()
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

let section = 'model-pricing'
const router = await import('@tanstack/react-router')
moduleMock.module('@tanstack/react-router', () => ({
  ...router,
  useParams: () => ({ section }),
}))
moduleMock.module('../components/settings-page', () => ({
  SettingsPage: () => {
    throw new Error(
      'Native prices must not use the partial legacy options view'
    )
  },
  SettingsPageFrame: ({ children }: { children: ReactNode }) => children,
}))
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider, notifyManager } =
  await import('@tanstack/react-query')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { BillingSettings } = await import('./index')
notifyManager.setScheduler(queueMicrotask)
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
const previous = useAuthStore.getState().auth
const adapter = api.defaults.adapter
let dispose: (() => Promise<void>) | undefined
afterEach(async () => {
  await dispose?.()
  dispose = undefined
  api.defaults.adapter = adapter
  useAuthStore.setState({ auth: previous })
  document.body.replaceChildren()
})
after(() => dom.close())

function configuration() {
  return {
    version: { routing: 3, pricing: 2 },
    secret_storage_enabled: false,
    routing: {
      version: 3,
      upstreams: [],
      groups: [{ name: 'default', enabled: true }],
      aliases: [],
      routes: [],
    },
    prices: {
      version: 2,
      rules: [
        {
          id: 19,
          model: 'imported-model',
          group: 'default',
          speed: 'standard',
          rates: {
            input: 2000000,
            output: 6000000,
            cache_read: 500000,
            cache_write: 0,
            request: 0,
          },
          group_multiplier_ppm: 750000,
          speed_multiplier_ppm: 1000000,
        },
      ],
    },
    bindings: {
      names: {},
      contexts: [],
      vendors: {},
      models: {},
      discoveries: {},
      price_drafts: {},
    },
  }
}
async function mount(nativeConfiguration = true, role = 100) {
  useAuthStore.getState().auth.setBundle({
    access_token: 'synthetic-pricing-session',
    token_type: 'Bearer',
    access_expires_at: 2_000_000_000,
    user: { id: 501, role, username: 'synthetic-admin' },
    session: {
      sid: 'synthetic-pricing-sid',
      current: true,
      login_method: 'password',
      ip: '127.0.0.1',
      user_agent: 'synthetic',
      created_at: 1,
      last_active_at: 1,
      expires_at: 2_000_000_000,
    },
  })
  const requests: string[] = []
  api.defaults.adapter = async (config) => {
    requests.push(config.url ?? '')
    assert.equal(config.method, 'get')
    const data =
      config.url === '/api/channel/ops'
        ? { native_configuration: nativeConfiguration, native_secrets: false }
        : config.url === '/core/v1/admin/routing/configuration'
          ? configuration()
          : assert.fail(`Unexpected request: ${config.url}`)
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: { success: true, data },
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
          <BillingSettings />
        </I18nextProvider>
      </QueryClientProvider>
    )
  )
  dispose = async () => {
    await act(async () => root.unmount())
    client.clear()
  }
  return { container, requests }
}
async function until(check: () => boolean) {
  for (let attempt = 0; attempt < 100 && !check(); attempt++) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 10))
    })
  }
  assert.ok(check(), 'Native pricing did not reach the expected state')
}
for (const entry of ['model-pricing', 'group-pricing']) {
  test(`${entry} reads published prices instead of partial legacy settings`, async () => {
    section = entry
    const { container, requests } = await mount()
    await until(() =>
      Boolean(
        container.querySelector('pre')?.textContent?.includes('imported-model')
      )
    )
    const published = JSON.parse(
      container.querySelector('pre')?.textContent ?? ''
    )
    assert.equal(published.version, 2)
    assert.equal(published.rules[0].rates.input, 2000000)
    assert.equal(published.rules[0].group_multiplier_ppm, 750000)
    assert.ok(requests.includes('/core/v1/admin/routing/configuration'))
    assert.ok(!requests.some((url) => url.startsWith('/api/option')))
  })
}
test('unavailable native configuration does not open a price editor', async () => {
  section = 'model-pricing'
  const { container, requests } = await mount(false)
  await until(
    () => container.textContent?.includes('Capability unavailable') === true
  )
  assert.ok(!requests.includes('/core/v1/admin/routing/configuration'))
  assert.equal(container.querySelector('textarea'), null)
})
test('an ordinary session cannot read native administrative prices', async () => {
  section = 'model-pricing'
  const { container, requests } = await mount(true, 1)
  await until(
    () =>
      container.textContent?.includes('Administrator access required') === true
  )
  assert.ok(!requests.includes('/core/v1/admin/routing/configuration'))
})
