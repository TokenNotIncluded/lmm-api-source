/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { Window } from 'happy-dom'

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
  HTMLInputElement: dom.HTMLInputElement,
  MutationObserver: dom.MutationObserver,
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
const { readNativeUsersScope } = await import('@/features/native-users/access')
const { PricingSourceSnapshot } = await import('./pricing-source-snapshot')
const {
  parsePricingSourceSnapshot,
  readPricingSourceSnapshot,
  serializePricingSourceSnapshot,
} = await import('./pricing-source-snapshot-api')
const { registerPricingSourceTranslations } =
  await import('./pricing-source-snapshot-i18n')
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

function fixture() {
  return {
    contract: 'core.legacy-pricing-snapshot.v1',
    purpose: 'reference_only',
    source_sha256: 'a'.repeat(64),
    observed_at: 1794852236,
    options: [
      {
        key: 'ModelRatio',
        value: '{ "synthetic-image": 1.2300e-7, "precise": 9007199254740993 }',
      },
      { key: 'GroupRatio', value: '{"default":0.7500}' },
    ],
    enabled_channels: [
      {
        id: 1,
        name: 'Source channel',
        models: 'synthetic-image',
        groups: 'default',
        model_mapping: '',
      },
    ],
    unsupported: [
      {
        model: 'synthetic-image',
        group: 'default',
        reason: 'media_contract_required',
        source_channel_ids: [1],
      },
    ],
  }
}
function session(role = 100, sid = 'synthetic-snapshot-session') {
  useAuthStore.getState().auth.setBundle({
    access_token: 'synthetic-snapshot-token',
    token_type: 'Bearer',
    access_expires_at: 2_000_000_000,
    user: { id: 501, role, username: 'synthetic-admin' },
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
async function mount(value: unknown = fixture(), role = 100) {
  session(role)
  const requests: string[] = []
  api.defaults.adapter = async (config) => {
    assert.equal(config.method, 'get')
    assert.equal(config.url, '/core/v1/admin/routing/pricing-source-snapshot')
    assert.deepEqual((config as { authScope: unknown }).authScope, {
      userId: 501,
      sessionId: 'synthetic-snapshot-session',
    })
    assert.ok(config.signal)
    requests.push(config.url)
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: { success: true, data: value },
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
          <PricingSourceSnapshot />
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
async function openPanel(container: HTMLElement) {
  const panel = container.querySelector('details')
  assert.ok(panel)
  await act(async () => {
    panel.open = true
    panel.dispatchEvent(new Event('toggle'))
  })
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 10))
  })
}
function required<T>(value: T | null | undefined): T {
  assert.notEqual(value, null)
  assert.notEqual(value, undefined)
  return value as T
}

test('snapshot schema preserves raw numeric text and rejects credentials, extra fields and unbound references', () => {
  const value = fixture()
  const snapshot = required(
    parsePricingSourceSnapshot({ success: true, data: value })
  )
  const downloaded = JSON.parse(serializePricingSourceSnapshot(snapshot))
  assert.deepEqual(downloaded, value)
  assert.match(downloaded.options[0].value, /1\.2300e-7/)
  assert.match(downloaded.options[0].value, /9007199254740993/)
  assert.equal(parsePricingSourceSnapshot({ success: true, data: null }), null)
  for (const bad of [
    { ...fixture(), purpose: 'active_billing' },
    { ...fixture(), secret: 'untrusted-secret' },
    {
      ...fixture(),
      options: [{ key: 'StripeSecretKey', value: 'untrusted-secret' }],
    },
    {
      ...fixture(),
      enabled_channels: [
        { ...fixture().enabled_channels[0], key: 'untrusted-secret' },
      ],
    },
    {
      ...fixture(),
      unsupported: [{ ...fixture().unsupported[0], source_channel_ids: [99] }],
    },
    {
      ...fixture(),
      options: [{ key: 'ModelRatio', value: '1'.repeat(512 * 1024 + 1) }],
    },
  ]) {
    assert.throws(() =>
      parsePricingSourceSnapshot({ success: true, data: bad })
    )
  }
})

test('panel loads only when opened, identifies reference-only values and filters without changing the export', async () => {
  const { container, requests } = await mount()
  assert.equal(requests.length, 0)
  assert.match(container.textContent ?? '', /Original configuration/)
  await openPanel(container)
  assert.equal(requests.length, 1)
  assert.match(
    container.textContent ?? '',
    /Only published native prices apply/
  )
  assert.match(container.textContent ?? '', /Unconverted model\/group pairs/)
  assert.equal(
    container.querySelector('time')?.dateTime,
    new Date(1794852236000).toISOString()
  )
  assert.ok(
    Array.from(container.querySelectorAll('pre')).some(
      (node) => node.textContent === fixture().options[0]?.value
    )
  )
  const input = required(container.querySelector('input'))
  await act(async () => {
    const setter = required(
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set
    )
    setter.call(input, 'GroupRatio')
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
  assert.equal(container.querySelectorAll('section pre').length, 1)
  assert.match(
    container.querySelector('section pre')?.textContent ?? '',
    /0.7500/
  )
  assert.ok(
    Array.from(container.querySelectorAll('button')).some(
      (button) => button.textContent === 'Download complete JSON'
    )
  )
  const snapshot = required(
    parsePricingSourceSnapshot({ success: true, data: fixture() })
  )
  assert.equal(
    JSON.parse(serializePricingSourceSnapshot(snapshot)).options.length,
    2
  )
})

test('missing and invalid snapshots show an honest empty state or error without exposing invalid data', async () => {
  const { container } = await mount(null)
  await openPanel(container)
  assert.match(
    container.textContent ?? '',
    /No original pricing snapshot is available/
  )
  await dispose?.()
  dispose = undefined
  const invalid = await mount({ ...fixture(), secret: 'DO-NOT-RENDER' })
  await openPanel(invalid.container)
  assert.match(invalid.container.textContent ?? '', /could not be loaded/)
  assert.doesNotMatch(
    invalid.container.textContent ?? '',
    /DO-NOT-RENDER|synthetic-image/
  )
})

test('non-administrators do not request the snapshot and a changed session cannot read it', async () => {
  const { container, requests } = await mount(fixture(), 1)
  assert.equal(container.textContent, '')
  assert.equal(requests.length, 0)
  await act(async () => session())
  const oldScope = required(readNativeUsersScope())
  await act(async () => session(100, 'replacement-session'))
  await assert.rejects(readPricingSourceSnapshot(oldScope), /Session changed/)
  assert.equal(requests.length, 0)
})

test('snapshot text is registered with equal, nonempty resources for all seven interface languages', () => {
  registerPricingSourceTranslations(i18n)
  const english = i18n.getResourceBundle(
    'en',
    'pricing-source-snapshot'
  ) as Record<string, string>
  for (const language of ['en', 'zh-CN', 'zh-TW', 'fr', 'ja', 'ru', 'vi']) {
    const resource = i18n.getResourceBundle(
      language,
      'pricing-source-snapshot'
    ) as Record<string, string>
    assert.deepEqual(Object.keys(resource).sort(), Object.keys(english).sort())
    assert.ok(
      Object.values(resource).every(
        (value) => typeof value === 'string' && value.length > 0
      )
    )
  }
})
