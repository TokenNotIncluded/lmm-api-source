/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { Window } from 'happy-dom'

const dom = new Window({
  url: 'https://console.example.test/system-settings/operations/hero-sms',
})
Reflect.deleteProperty(dom.Element.prototype, 'animate')
for (const name of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLInputElement',
  'HTMLButtonElement',
  'Node',
  'Element',
  'Event',
  'MouseEvent',
  'MutationObserver',
  'ResizeObserver',
  'DocumentFragment',
  'ShadowRoot',
  'getComputedStyle',
  'matchMedia',
  'requestAnimationFrame',
  'cancelAnimationFrame',
] as const) {
  Object.defineProperty(globalThis, name, {
    configurable: true,
    value: dom[name],
  })
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
const { HeroSmsSettingsSection } = await import('./hero-sms-settings-section')
notifyManager.setScheduler(queueMicrotask)
const i18n = createInstance()
await i18n.use(initReactI18next).init({ lng: 'en', resources: {} })
const priorAdapter = api.defaults.adapter
let dispose: (() => Promise<void>) | undefined

afterEach(async () => {
  await dispose?.()
  dispose = undefined
  api.defaults.adapter = priorAdapter
  document.body.replaceChildren()
})
after(() => dom.close())

const settings = {
  revision: '9007199254740993',
  enabled: true,
  email_enabled: true,
  sms_enabled: false,
  api_key_configured: true,
  credential_managed_by_file: true,
  pending_work: false,
  purchases_available: false,
  currency: 'USD',
  currency_code: 840,
  price_multiplier: '10',
}

async function mount() {
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: 1, retryDelay: 0 },
    },
  })
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  await act(async () =>
    root.render(
      <QueryClientProvider client={client}>
        <I18nextProvider i18n={i18n}>
          <HeroSmsSettingsSection />
        </I18nextProvider>
      </QueryClientProvider>
    )
  )
  dispose = async () => {
    await act(async () => root.unmount())
    client.clear()
  }
  return container
}

async function until(check: () => boolean) {
  for (let attempt = 0; attempt < 50 && !check(); attempt++) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 10))
    })
  }
  assert.ok(check(), 'HeroSMS settings did not reach the expected state')
}

function button(text: string) {
  const result = Array.from(
    document.querySelectorAll<HTMLButtonElement>('button')
  ).find((element) => element.textContent?.trim() === text)
  assert.ok(result, `Missing button: ${text}`)
  return result
}

test('an enabled legacy policy exposes only the read-only purchase capability and host credential test', async () => {
  const requests: { method?: string; url?: string; body?: unknown }[] = []
  api.defaults.adapter = async (config) => {
    requests.push({
      method: config.method,
      url: config.url,
      body: config.data && JSON.parse(config.data),
    })
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: {
        success: true,
        data: config.method === 'post' ? { ok: true } : settings,
      },
    }
  }
  const container = await mount()
  await until(
    () => container.textContent?.includes('Purchases unavailable') === true
  )
  assert.match(
    container.textContent ?? '',
    /HeroSMS API key is managed on the hostConfigured/
  )
  assert.equal(container.querySelector('input, [role="switch"], form'), null)
  assert.deepEqual(
    Array.from(container.querySelectorAll('button'), (b) => b.textContent),
    ['Test connection']
  )
  await act(async () => button('Test connection').click())
  await until(
    () =>
      container
        .querySelector('[role="status"]')
        ?.textContent?.includes('The server can reach HeroSMS') === true
  )
  assert.deepEqual(requests, [
    { method: 'get', url: '/api/option/hero-sms', body: undefined },
    { method: 'post', url: '/api/option/hero-sms/test', body: {} },
  ])
})

test('a missing host credential disables the diagnostic', async () => {
  api.defaults.adapter = async (config) => ({
    config,
    headers: {},
    status: 200,
    statusText: 'OK',
    data: { success: true, data: { ...settings, api_key_configured: false } },
  })
  const container = await mount()
  await until(() => container.textContent?.includes('Not configured') === true)
  assert.equal(button('Test connection').disabled, true)
})

test('an unreadable status offers retry without showing a guessed capability', async () => {
  let reads = 0
  api.defaults.adapter = async (config) => ({
    config,
    headers: {},
    status: 200,
    statusText: 'OK',
    data: { success: ++reads > 1, data: settings },
  })
  const container = await mount()
  await until(
    () =>
      container.textContent?.includes('Unable to load HeroSMS settings') ===
      true
  )
  assert.equal(container.textContent?.includes('Purchases unavailable'), false)
  await act(async () => button('Retry').click())
  await until(
    () => container.textContent?.includes('Purchases unavailable') === true
  )
  assert.equal(reads, 2)
})
