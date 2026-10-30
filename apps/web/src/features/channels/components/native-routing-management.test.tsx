/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { Window } from 'happy-dom'

const dom = new Window()
for (const name of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'Element',
  'Node',
  'Event',
  'MouseEvent',
] as const) {
  Object.defineProperty(globalThis, name, {
    configurable: true,
    value: name === 'window' ? dom : dom[name],
  })
}
Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
  configurable: true,
  value: true,
})
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { NativeRoutingManagement } = await import('./native-routing-management')
const request = api.request
const previous = useAuthStore.getState().auth
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
let dispose: (() => Promise<void>) | undefined
afterEach(async () => {
  await dispose?.()
  dispose = undefined
  api.request = request
  useAuthStore.setState({ auth: previous })
  document.body.replaceChildren()
})
after(() => dom.close())
function login(sid = 'synthetic-routing-editor', role = 10) {
  useAuthStore.getState().auth.setBundle({
    access_token: 'synthetic-channel-session-token',
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
function fixture() {
  return {
    version: { routing: 1, pricing: 1 },
    secret_storage_enabled: true,
    routing: {
      version: 1,
      upstreams: [
        {
          id: 1,
          endpoint: 'https://private-provider.example/v1/',
          protocol: 'openai_chat',
          credential: { id: 7, revision: 1 },
          enabled: true,
        },
      ],
      groups: [{ name: 'default', enabled: true }],
      aliases: [],
      routes: [
        {
          id: 1,
          model: 'model-a',
          group: 'default',
          enabled: true,
          speeds: ['standard'],
          targets: [
            {
              id: 1,
              upstream_id: 1,
              upstream_model: 'model-a',
              enabled: true,
              priority: 0,
              weight: 1,
              speeds: ['standard'],
              payload: 'adapt',
              timeout_ms: 30_000,
            },
          ],
          failover: {
            max_attempts: 1,
            retry_on: [],
            allow_replay_after_send: false,
          },
        },
      ],
    },
    prices: {
      version: 1,
      rules: [
        {
          id: 1,
          model: 'model-a',
          group: 'default',
          speed: 'standard',
          rates: {
            input: 2,
            output: 4,
            cache_read: 0,
            cache_write: 0,
            request: 0,
          },
          group_multiplier_ppm: 1_000_000,
          speed_multiplier_ppm: 1_000_000,
        },
      ],
    },
    bindings: {
      names: { 1: 'Private provider' },
      contexts: [],
      vendors: {},
      models: {},
      discoveries: {},
      price_drafts: {},
    },
  }
}
async function mount(initialSection: 'channels' | 'prices') {
  login()
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
          <NativeRoutingManagement initialSection={initialSection} />
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
  for (let attempt = 0; attempt < 100; attempt++) {
    if (check()) return
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 10))
    })
  }
  assert.ok(
    check(),
    'The native management view did not reach the expected state'
  )
}
function button(container: Element, text: string) {
  const value = [...container.querySelectorAll('button')].find(
    (element) => element.textContent === text
  )
  assert.ok(value, `Missing button ${text}`)
  return value as HTMLButtonElement
}

test('prices require a reviewed preview and editing invalidates the exact confirmation before one CAS publication', async () => {
  let config = fixture()
  const writes: Array<{ path: string; body: Record<string, unknown> }> = []
  let previews = 0
  api.request = (async (options: {
    url: string
    method: string
    data?: Record<string, unknown>
  }) => {
    if (options.method === 'GET') {
      return { data: { success: true, data: config } }
    }
    assert.ok(options.data)
    if (options.url.endsWith('/preview')) {
      previews++
      return {
        data: {
          success: true,
          data: {
            saved: false,
            after: options.data.prices,
            confirmation_digest: String(previews).repeat(64),
          },
        },
      }
    }
    writes.push({ path: options.url, body: options.data })
    config = {
      ...config,
      version: { routing: 2, pricing: 2 },
      routing: { ...config.routing, version: 2 },
      prices: options.data.prices as typeof config.prices,
    }
    return { data: { success: true, data: { version: config.version } } }
  }) as typeof api.request
  const container = await mount('prices')
  await until(() =>
    [...container.querySelectorAll('button')].some(
      (element) => element.textContent === 'Edit'
    )
  )
  await act(async () => button(container, 'Edit').click())
  assert.equal(container.textContent?.includes('Confirm and apply'), false)
  await act(async () => button(container, 'Preview').click())
  await until(() => !!container.textContent?.includes('Confirm and apply'))
  assert.equal(writes.length, 0)
  const textarea = container.querySelector('textarea')
  assert.ok(textarea)
  const data = JSON.parse(textarea.value)
  data.prices.rules[0].rates.input = 10
  const setter = Object.getOwnPropertyDescriptor(
    dom.HTMLTextAreaElement.prototype,
    'value'
  )?.set
  assert.ok(setter)
  await act(async () => {
    setter.call(textarea, JSON.stringify(data))
    textarea.dispatchEvent(new Event('input', { bubbles: true }))
  })
  assert.equal(container.textContent?.includes('Confirm and apply'), false)
  await act(async () => button(container, 'Preview').click())
  await until(() => !!container.textContent?.includes('Confirm and apply'))
  await act(async () => button(container, 'Confirm and apply').click())
  await until(() => writes.length === 1 && !container.querySelector('textarea'))
  assert.equal(previews, 2)
  assert.equal(writes[0].path, '/core/v1/admin/routing/prices')
  assert.deepEqual(writes[0].body.expected_version, { routing: 1, pricing: 1 })
  assert.equal(writes[0].body.confirmation_digest, '2'.repeat(64))
  assert.equal(
    (writes[0].body.prices as typeof config.prices).rules[0].rates.input,
    10
  )
})

test('private channel data and an unsaved secret disappear when the administrator session changes', async () => {
  let reads = 0
  api.request = (async () => {
    reads++
    return { data: { success: true, data: fixture() } }
  }) as typeof api.request
  const container = await mount('channels')
  await until(
    () => !!container.textContent?.includes('private-provider.example')
  )
  await act(async () => button(container, 'Create Channel').click())
  const secret = container.querySelector('input[type=password]')
  assert.ok(secret)
  await act(async () => login('ordinary-replacement', 1))
  await until(
    () => !!container.textContent?.includes('Administrator access required')
  )
  assert.equal(container.querySelector('textarea'), null)
  assert.equal(container.querySelector('input[type=password]'), null)
  assert.equal(
    container.textContent?.includes('private-provider.example'),
    false
  )
  assert.equal(reads, 1)
  await act(async () => login('another-admin', 10))
  await until(
    () =>
      reads === 2 &&
      !!container.textContent?.includes('private-provider.example')
  )
})
