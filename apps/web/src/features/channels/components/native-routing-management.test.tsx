/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { Window } from 'happy-dom'

import type { NativeConfiguration } from '../native-api'

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
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { NativeRoutingManagement } = await import('./native-routing-management')
const adapter = api.defaults.adapter
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
  api.defaults.adapter = adapter
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
  const names: NativeConfiguration['bindings']['names'] = {
    1: 'Private provider',
  }
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
      names,
      contexts: [],
      vendors: {},
      models: {},
      discoveries: {},
      price_drafts: {},
    },
  }
}
async function mount(
  initialSection: 'channels' | 'prices',
  nativeSecrets = true
) {
  api.defaults.adapter = async (config) => {
    assert.equal(config.url, '/api/channel/ops')
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: { success: true, data: { native_secrets: nativeSecrets } },
    }
  }
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

async function openCopy(container: Element) {
  await until(() =>
    [...container.querySelectorAll('button')].some(
      (item) => item.textContent === 'Copy Channel'
    )
  )
  await act(async () => button(container, 'Copy Channel').click())
  await until(() => !!document.querySelector('[role=dialog]'))
  const dialog = document.querySelector('[role=dialog]')
  assert.ok(dialog)
  return dialog
}
async function changeSuffix(dialog: Element, suffix: string) {
  const input = dialog.querySelector('input')
  assert.ok(input)
  const setter = Object.getOwnPropertyDescriptor(
    dom.HTMLInputElement.prototype,
    'value'
  )?.set
  assert.ok(setter)
  await act(async () => {
    setter.call(input, suffix)
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
  return input
}

test('native copy previews the suffix without counters or secrets, blocks double dispatch and refreshes the published channel', async () => {
  let config = fixture()
  let reads = 0
  let writes = 0
  let complete: (() => void) | undefined
  api.request = (async (options: {
    method: string
    url: string
    data?: unknown
  }) => {
    if (options.method === 'GET') {
      reads++
      return { data: { success: true, data: config } }
    }
    writes++
    assert.equal(options.url, '/core/v1/admin/routing/channels/1/copy')
    assert.deepEqual(options.data, {
      expected_version: { routing: 1, pricing: 1 },
      suffix: '_copy',
    })
    await new Promise<void>((resolve) => {
      complete = resolve
    })
    config = {
      ...config,
      version: { routing: 2, pricing: 1 },
      routing: {
        ...config.routing,
        version: 2,
        upstreams: [
          ...config.routing.upstreams,
          { ...config.routing.upstreams[0], id: 2 },
        ],
      },
      bindings: {
        ...config.bindings,
        names: { ...config.bindings.names, 2: 'Private provider_copy' },
      },
    }
    return { data: { success: true, data: { id: 2, version: config.version } } }
  }) as typeof api.request
  const container = await mount('channels')
  const dialog = await openCopy(container)
  assert.equal(dialog.querySelector('input')?.value, '_copy')
  assert.ok(dialog.textContent?.includes('Private provider_copy'))
  assert.equal(dialog.querySelector('[role=checkbox]'), null)
  assert.equal(dialog.querySelector('input[type=password]'), null)
  await act(async () => {
    button(dialog, 'Copy Channel').click()
    button(dialog, 'Copy Channel').click()
  })
  assert.equal(writes, 1)
  assert.equal(button(dialog, 'Copying...').disabled, true)
  await act(async () => complete?.())
  await until(
    () =>
      reads >= 2 &&
      [...container.querySelectorAll('h3')].some(
        (item) => item.textContent === 'Private provider_copy'
      )
  )
  assert.equal(writes, 1)
})

test('native copy preserves suffixes on failure and resets the draft after an administrative session change', async () => {
  let writes = 0
  api.request = (async (options: { method: string; data?: unknown }) => {
    if (options.method === 'GET') {
      return { data: { success: true, data: fixture() } }
    }
    writes++
    assert.deepEqual(options.data, {
      expected_version: { routing: 1, pricing: 1 },
      suffix: writes === 1 ? '_副本' : '',
    })
    return { data: { success: false, message: 'Copy denied', data: {} } }
  }) as typeof api.request
  const container = await mount('channels')
  const dialog = await openCopy(container)
  const input = await changeSuffix(dialog, '_副本')
  await act(async () => button(dialog, 'Copy Channel').click())
  await until(() => !!dialog.textContent?.includes('Copy denied'))
  assert.equal(input.value, '_副本')
  await changeSuffix(dialog, '')
  await act(async () => button(dialog, 'Copy Channel').click())
  await until(() => writes === 2 && !button(dialog, 'Copy Channel').disabled)
  assert.equal(input.value, '')
  await act(async () => login('replacement-copy-admin'))
  await until(() => !document.querySelector('[role=dialog]'))
  const reopened = await openCopy(container)
  assert.equal(reopened.querySelector('input')?.value, '_copy')
  assert.equal(writes, 2)
})

test('native copy stays visible but disabled when native secret capability or configured key storage is unavailable', async () => {
  for (const storageEnabled of [true, false]) {
    const config = { ...fixture(), secret_storage_enabled: storageEnabled }
    let writes = 0
    api.request = (async (options: { method: string }) => {
      if (options.method !== 'GET') writes++
      return { data: { success: true, data: config } }
    }) as typeof api.request
    const container = await mount('channels', !storageEnabled)
    await until(() =>
      [...container.querySelectorAll('button')].some(
        (item) => item.textContent === 'Copy Channel'
      )
    )
    assert.equal(button(container, 'Copy Channel').disabled, true)
    assert.ok(
      container.textContent?.includes('API Keys: Capability unavailable')
    )
    await act(async () => button(container, 'Copy Channel').click())
    assert.equal(document.querySelector('[role=dialog]'), null)
    assert.equal(writes, 0)
    await dispose?.()
    dispose = undefined
    document.body.replaceChildren()
  }
})

test('native copy previews the generated name for an unnamed bootstrap channel', async () => {
  const config = fixture()
  api.request = (async () => ({
    data: {
      success: true,
      data: { ...config, bindings: { ...config.bindings, names: {} } },
    },
  })) as typeof api.request
  const container = await mount('channels')
  const dialog = await openCopy(container)
  assert.match(dialog.textContent ?? '', /Channel 1_copy/)
})
