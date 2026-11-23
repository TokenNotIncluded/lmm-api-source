/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { Window } from 'happy-dom'

import {
  parseNativeDocument,
  stringifyNativeDocument,
  type NativeConfiguration,
} from '../native-api'

type SuccessfulResponse<Data> = { data: { success: true; data: Data } }

const dom = new Window({
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
  HTMLButtonElement: dom.HTMLButtonElement,
  DocumentFragment: dom.DocumentFragment,
  ShadowRoot: dom.ShadowRoot,
  requestAnimationFrame: dom.requestAnimationFrame.bind(dom),
  cancelAnimationFrame: dom.cancelAnimationFrame.bind(dom),
  getComputedStyle: dom.getComputedStyle.bind(dom),
  scrollTo: dom.scrollTo.bind(dom),
})) {
  Object.defineProperty(globalThis, name, { configurable: true, value })
}
Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
  configurable: true,
  writable: true,
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
const { Channels } = await import('../index')
const {
  createRootRoute,
  createRoute,
  createRouter,
  createMemoryHistory,
  RouterProvider,
  Outlet,
} = await import('@tanstack/react-router')
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
const accountingCapabilities = {
  native_channel_usage: true,
  native_channel_counters: true,
  native_balance_probe: true,
  copy_preserve_counters: true,
}
async function mount(
  initialSection: 'channels' | 'models' | 'vendors' | 'prices',
  nativeSecrets = true,
  accounting: Partial<typeof accountingCapabilities> = accountingCapabilities,
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } }),
  channelPage = false,
  configurationWritable = true
) {
  api.defaults.adapter = async (config) => {
    if (config.url === '/api/channel') {
      return {
        config,
        headers: {},
        status: 200,
        statusText: 'OK',
        data: { success: true, data: { items: [], total: 0 } },
      }
    }
    assert.equal(config.url, '/api/channel/ops')
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: {
        success: true,
        data: {
          native_secrets: nativeSecrets,
          bulk_key_import: nativeSecrets,
          native_configuration: true,
          ...accounting,
        },
      },
    }
  }
  login()
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  const rootRoute = createRootRoute({ component: Outlet })
  const authRoute = createRoute({
    getParentRoute: () => rootRoute,
    id: '_authenticated',
    component: Outlet,
  })
  const channelsRoute = createRoute({
    getParentRoute: () => authRoute,
    path: '/channels/',
    validateSearch: (search: Record<string, unknown>) => ({
      filter: typeof search.filter === 'string' ? search.filter : '',
      page: typeof search.page === 'number' ? search.page : 1,
    }),
    component: Channels,
  })
  const router = createRouter({
    routeTree: rootRoute.addChildren([authRoute.addChildren([channelsRoute])]),
    history: createMemoryHistory({ initialEntries: ['/channels/'] }),
  })
  if (channelPage) await router.load()
  await act(async () =>
    root.render(
      <QueryClientProvider client={client}>
        <I18nextProvider i18n={i18n}>
          <>
            {channelPage ? (
              <RouterProvider router={router} />
            ) : (
              <NativeRoutingManagement
                initialSection={initialSection}
                configurationWritable={configurationWritable}
              />
            )}
          </>
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
  let config = { ...fixture(), channel_id_high_water: 17 }
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
    assert.equal(Object.hasOwn(options.data, 'channel_id_high_water'), false)
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

test('twenty exact tariff fractions survive price review, publication and subsequent visual model edits', async () => {
  const base = fixture()
  const literals = Array.from({ length: 20 }, (_, index) =>
    (9_007_199_254_740_993n + BigInt(index * 2)).toString()
  )
  const rows = literals.map((literal, index) => {
    const row = JSON.stringify({
      ...base.prices.rules[0],
      id: index + 1,
      model: index === 0 ? 'model-a' : `catalog-${index}`,
      rates: { input: 0, output: 0, cache_read: 0, cache_write: 0, request: 0 },
    })
    return (
      `${row.slice(
        0,
        -1
      )},"billing_mode":"tiered_decimal","tiered":{"revision":1,"source_digest":"${'a'.repeat(
        64
      )}","uses_cache_read":false,"uses_cache_write":false,"tiers":[{"rates":` +
      `{"input":{"numerator":${literal},"denominator":1000000000000000000},` +
      `"output":{"numerator":1,"denominator":1},` +
      `"cache_read":{"numerator":0,"denominator":1},` +
      `"cache_write":{"numerator":0,"denominator":1}}}]}}`
    )
  })
  let config: NativeConfiguration = {
    ...base,
    prices: parseNativeDocument(
      `{"prices":{"version":1,"rules":[${rows.join(',')}]}}`
    ).prices as NativeConfiguration['prices'],
  }
  let published = false
  api.request = (async (options: {
    url: string
    method: string
    data?: Record<string, unknown>
  }) => {
    if (options.method === 'GET') {
      return { data: { success: true, data: config } }
    }
    assert.ok(options.data)
    const wire = stringifyNativeDocument(options.data)
    for (const literal of literals) {
      assert.ok(wire.includes(`"numerator":${literal},`))
    }
    if (options.url.endsWith('/preview')) {
      return {
        data: {
          success: true,
          data: {
            after: options.data.prices ?? options.data.model,
            confirmation_digest: 'b'.repeat(64),
          },
        },
      }
    }
    assert.equal(options.data.confirmation_digest, 'b'.repeat(64))
    published = true
    config = {
      ...config,
      version: { routing: 2, pricing: 2 },
      routing: { ...config.routing, version: 2 },
      prices: options.data.prices as NativeConfiguration['prices'],
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
  const textarea = required(container.querySelector('textarea'))
  for (const literal of literals) assert.ok(textarea.value.includes(literal))
  assert.equal(textarea.value.includes('isLosslessNumber'), false)
  await act(async () => button(container, 'Preview').click())
  await until(
    () => container.textContent?.includes('Confirm and apply') === true
  )
  await act(async () => button(container, 'Confirm and apply').click())
  await until(() => published && !container.querySelector('textarea'))
  await act(async () => button(container, 'Models').click())
  await act(async () => button(container, 'Edit').click())
  const name = required(
    [...container.querySelectorAll('label')]
      .find((label) => label.textContent?.includes('Display name'))
      ?.querySelector('input')
  )
  await changeInput(name, 'Reviewed exact tariff')
  const edited = required(
    [...container.querySelectorAll('label')]
      .find((label) => label.textContent?.includes('Configuration'))
      ?.querySelector('textarea')
  )
  assert.ok(edited.value.includes(literals[0]))
  assert.equal(edited.value.includes('isLosslessNumber'), false)
  const modelDraft = stringifyNativeDocument(parseNativeDocument(edited.value))
  assert.ok(modelDraft.includes(`"numerator":${literals[0]},`))
  assert.ok(modelDraft.includes('Reviewed exact tariff'))
})

test('native channel cards toggle status once with CAS and refresh the published state', async () => {
  let config = fixture()
  const writes: Array<Record<string, unknown>> = []
  api.request = (async (options: {
    method: string
    url: string
    data?: Record<string, unknown>
  }) => {
    if (options.method === 'GET') {
      return { data: { success: true, data: config } }
    }
    assert.equal(options.url, '/api/channel/1/status')
    assert.deepEqual(options.data, {
      enabled: writes.length === 0 ? false : true,
      expected_version:
        writes.length === 0
          ? { routing: 1, pricing: 1 }
          : { routing: 2, pricing: 1 },
    })
    writes.push(options.data ?? {})
    const enabled = writes.length !== 1
    config = {
      ...config,
      version: { routing: config.version.routing + 1, pricing: 1 },
      routing: {
        ...config.routing,
        version: config.routing.version + 1,
        upstreams: [{ ...config.routing.upstreams[0], enabled }],
      },
    }
    return { data: { success: true, data: { version: config.version } } }
  }) as typeof api.request
  const container = await mount('channels')
  await until(() =>
    [...container.querySelectorAll('button')].some(
      (element) => element.textContent === 'Disable'
    )
  )
  assert.ok(container.textContent?.includes('#1 · Enabled'))
  assert.equal(button(container, 'Disable').closest('details'), null)
  assert.equal(button(container, 'API Keys').closest('details'), null)
  await act(async () => {
    button(container, 'Disable').click()
    button(container, 'Disable').click()
  })
  await until(
    () =>
      writes.length === 1 &&
      [...container.querySelectorAll('button')].some(
        (element) => element.textContent === 'Enable'
      )
  )
  assert.ok(container.textContent?.includes('#1 · Disabled'))
  await act(async () => button(container, 'Enable').click())
  await until(
    () =>
      writes.length === 2 &&
      [...container.querySelectorAll('button')].some(
        (element) => element.textContent === 'Disable'
      )
  )
  assert.ok(container.textContent?.includes('#1 · Enabled'))
})

test('native channel selection batches status changes and expires with the published snapshot', async () => {
  const base = fixture()
  let config = {
    ...base,
    routing: {
      ...base.routing,
      upstreams: [
        ...base.routing.upstreams,
        { ...base.routing.upstreams[0], id: 2 },
      ],
    },
    bindings: {
      ...base.bindings,
      names: { ...base.bindings.names, 2: 'Secondary provider' },
    },
  }
  const writes: Array<Record<string, unknown>> = []
  api.request = (async (options: {
    method: string
    url: string
    data?: Record<string, unknown>
  }) => {
    if (options.method === 'GET') {
      return { data: { success: true, data: config } }
    }
    assert.equal(options.url, '/api/channel/status/batch')
    writes.push(options.data ?? {})
    const ids = options.data?.ids as number[]
    const enabled = options.data?.enabled as boolean
    config = {
      ...config,
      version: { routing: config.version.routing + 1, pricing: 1 },
      routing: {
        ...config.routing,
        version: config.routing.version + 1,
        upstreams: config.routing.upstreams.map((channel) =>
          ids.includes(channel.id) ? { ...channel, enabled } : channel
        ),
      },
    }
    return { data: { success: true, data: { version: config.version } } }
  }) as typeof api.request
  const container = await mount('channels')
  await until(() => container.querySelectorAll('[role=checkbox]').length === 3)
  await act(async () =>
    (container.querySelector('[role=checkbox]') as HTMLButtonElement).click()
  )
  await until(() => container.textContent?.includes('2 selected') === true)
  let batch = required(
    container.querySelector('[data-slot=channel-batch-actions]')
  )
  await act(async () => {
    button(batch, 'Disable').click()
    button(batch, 'Disable').click()
  })
  await until(
    () =>
      writes.length === 1 &&
      !container.querySelector('[data-slot=channel-batch-actions]')
  )
  assert.deepEqual(writes[0], {
    ids: [1, 2],
    enabled: false,
    expected_version: { routing: 1, pricing: 1 },
  })
  const secondary = container.querySelectorAll('[role=checkbox]')[2]
  await act(async () => (secondary as HTMLButtonElement).click())
  await until(() => container.textContent?.includes('1 selected') === true)
  batch = required(container.querySelector('[data-slot=channel-batch-actions]'))
  await act(async () => button(batch, 'Enable').click())
  await until(
    () =>
      writes.length === 2 &&
      !container.querySelector('[data-slot=channel-batch-actions]')
  )
  assert.deepEqual(writes[1], {
    ids: [2],
    enabled: true,
    expected_version: { routing: 2, pricing: 1 },
  })
})

test('native batch routing submits selected channels once with only valid changed fields', async () => {
  const base = fixture()
  let config = {
    ...base,
    model_limits: { total_timeout_ms: 120_000 },
    routing: {
      ...base.routing,
      upstreams: [
        ...base.routing.upstreams,
        { ...base.routing.upstreams[0], id: 2 },
      ],
    },
    bindings: {
      ...base.bindings,
      names: { ...base.bindings.names, 2: 'Secondary provider' },
    },
  }
  const writes: Array<Record<string, unknown>> = []
  api.request = (async (options: {
    method: string
    url: string
    data?: Record<string, unknown>
  }) => {
    if (options.method === 'GET') {
      return { data: { success: true, data: config } }
    }
    assert.equal(options.method, 'PUT')
    assert.equal(
      options.url,
      '/core/v1/admin/routing/channels/batch/configuration'
    )
    writes.push(options.data ?? {})
    config = {
      ...config,
      version: { routing: 2, pricing: 1 },
      routing: { ...config.routing, version: 2 },
    }
    return {
      data: {
        success: true,
        data: { version: config.version, updated: 2, target_count: 1 },
      },
    }
  }) as typeof api.request
  const container = await mount('channels')
  await until(() => container.querySelectorAll('[role=checkbox]').length === 3)
  await act(async () =>
    (container.querySelector('[role=checkbox]') as HTMLButtonElement).click()
  )
  const batch = required(
    container.querySelector('[data-slot=channel-batch-actions]')
  )
  await act(async () => button(batch, 'Edit routing').click())
  const form = required(
    container.querySelector('form[aria-label="Edit routing"]')
  )
  assert.equal(button(form, 'Apply').disabled, true)
  await changeInput(required(form.querySelector('input[name=priority]')), '0')
  await changeInput(required(form.querySelector('input[name=weight]')), '1.5')
  assert.equal(button(form, 'Apply').disabled, true)
  assert.match(
    form.textContent ?? '',
    /Enter whole numbers within the allowed ranges/
  )
  assert.equal(writes.length, 0)
  await changeInput(required(form.querySelector('input[name=weight]')), '')
  await changeInput(required(form.querySelector('input[name=timeout_ms]')), '0')
  assert.equal(button(form, 'Apply').disabled, true)
  assert.match(
    form.textContent ?? '',
    /Target timeout must be at least 120000 ms/
  )
  assert.match(form.textContent ?? '', /larger values do not extend execution/)
  assert.equal(
    required(form.querySelector<HTMLInputElement>('input[name=timeout_ms]'))
      .min,
    '120000'
  )
  await changeInput(
    required(form.querySelector('input[name=timeout_ms]')),
    '120000'
  )
  assert.equal(button(form, 'Apply').disabled, false)
  await act(async () => {
    button(form, 'Apply').click()
    button(form, 'Apply').click()
  })
  await until(
    () =>
      writes.length === 1 &&
      !container.querySelector('[data-slot=channel-batch-actions]')
  )
  assert.deepEqual(writes[0], {
    expected_version: { routing: 1, pricing: 1 },
    ids: [1, 2],
    patch: { priority: 0, timeout_ms: 120000 },
  })
  assert.equal(container.querySelector('form[aria-label="Edit routing"]'), null)
})

test('native batch routing preserves a failed draft without replay and discards it when selection changes', async () => {
  const config = fixture()
  const writes: Array<Record<string, unknown>> = []
  api.request = (async (options: {
    method: string
    data?: Record<string, unknown>
  }) => {
    if (options.method === 'GET') {
      return { data: { success: true, data: config } }
    }
    writes.push(options.data ?? {})
    return { data: { success: false, message: 'Routing save failed' } }
  }) as typeof api.request
  const container = await mount('channels')
  await until(() => container.querySelectorAll('[role=checkbox]').length === 2)
  const selection = container.querySelector(
    '[role=checkbox]'
  ) as HTMLButtonElement
  await act(async () => selection.click())
  const batch = required(
    container.querySelector('[data-slot=channel-batch-actions]')
  )
  await act(async () => button(batch, 'Edit routing').click())
  const form = required(
    container.querySelector('form[aria-label="Edit routing"]')
  )
  const weight = required(
    form.querySelector<HTMLInputElement>('input[name=weight]')
  )
  await changeInput(weight, '0')
  await act(async () => button(form, 'Apply').click())
  await until(
    () => container.textContent?.includes('Routing save failed') === true
  )
  assert.deepEqual(writes, [
    {
      expected_version: { routing: 1, pricing: 1 },
      ids: [1],
      patch: { weight: 0 },
    },
  ])
  assert.equal(weight.value, '0')
  assert.equal(button(form, 'Apply').disabled, false)
  await act(async () => selection.click())
  assert.equal(container.querySelector('form[aria-label="Edit routing"]'), null)
  await act(async () => selection.click())
  assert.equal(container.querySelector('form[aria-label="Edit routing"]'), null)
  const nextBatch = required(
    container.querySelector('[data-slot=channel-batch-actions]')
  )
  await act(async () => button(nextBatch, 'Edit routing').click())
  const reopened = required(
    container.querySelector('form[aria-label="Edit routing"]')
  )
  assert.equal(
    required(reopened.querySelector<HTMLInputElement>('input[name=weight]'))
      .value,
    ''
  )
  assert.equal(button(reopened, 'Apply').disabled, true)
  assert.equal(writes.length, 1)
})

test('native batch routing rejects a stale version without replaying against the refreshed configuration', async () => {
  let config = fixture()
  const writes: Array<Record<string, unknown>> = []
  api.request = (async (options: {
    method: string
    data?: Record<string, unknown>
  }) => {
    if (options.method === 'GET') {
      return { data: { success: true, data: config } }
    }
    writes.push(options.data ?? {})
    config = {
      ...config,
      version: { routing: 2, pricing: 1 },
      routing: { ...config.routing, version: 2 },
    }
    return { data: { success: false, message: 'Routing version conflict' } }
  }) as typeof api.request
  const container = await mount('channels')
  await until(() => container.querySelectorAll('[role=checkbox]').length === 2)
  await act(async () =>
    (container.querySelector('[role=checkbox]') as HTMLButtonElement).click()
  )
  const batch = required(
    container.querySelector('[data-slot=channel-batch-actions]')
  )
  await act(async () => button(batch, 'Edit routing').click())
  const form = required(
    container.querySelector('form[aria-label="Edit routing"]')
  )
  await changeInput(required(form.querySelector('input[name=priority]')), '10')
  await act(async () => button(form, 'Apply').click())
  await until(
    () => container.textContent?.includes('Routing version conflict') === true
  )
  await until(
    () => !container.querySelector('[data-slot=channel-batch-actions]')
  )
  assert.equal(container.querySelector('form[aria-label="Edit routing"]'), null)
  assert.deepEqual(writes, [
    {
      expected_version: { routing: 1, pricing: 1 },
      ids: [1],
      patch: { priority: 10 },
    },
  ])
})

test('native batch tags preserve drafts on failure and clear only selected channels', async () => {
  const base = fixture()
  let config = {
    ...base,
    routing: {
      ...base.routing,
      upstreams: [
        ...base.routing.upstreams,
        { ...base.routing.upstreams[0], id: 2 },
      ],
    },
    bindings: {
      ...base.bindings,
      names: { ...base.bindings.names, 2: 'Secondary provider' },
      tags: {} as Record<number, string>,
    },
  }
  const writes: Array<Record<string, unknown>> = []
  api.request = (async (options: {
    method: string
    url: string
    data?: Record<string, unknown>
  }) => {
    if (options.method === 'GET') {
      return { data: { success: true, data: config } }
    }
    assert.equal(options.url, '/core/v1/admin/routing/channels/batch/tag')
    writes.push(options.data ?? {})
    if (writes.length === 2) {
      return { data: { success: false, message: 'Tag conflict', data: {} } }
    }
    const ids = options.data?.ids as number[]
    const tag = options.data?.tag as string | null
    const tags = { ...config.bindings.tags }
    for (const id of ids) {
      if (tag === null) delete tags[id]
      else tags[id] = tag
    }
    config = {
      ...config,
      version: { routing: config.version.routing + 1, pricing: 1 },
      routing: { ...config.routing, version: config.routing.version + 1 },
      bindings: { ...config.bindings, tags },
    }
    return {
      data: {
        success: true,
        data: { updated: ids.length, version: config.version },
      },
    }
  }) as typeof api.request
  const container = await mount('channels')
  await until(() => container.querySelectorAll('[role=checkbox]').length === 3)
  await act(async () =>
    (container.querySelector('[role=checkbox]') as HTMLButtonElement).click()
  )
  await until(() => container.textContent?.includes('2 selected') === true)
  let batch = required(
    container.querySelector('[data-slot=channel-batch-actions]')
  )
  await act(async () => button(batch, 'Tag').click())
  let form = required(
    container.querySelector('form[aria-label="Set tag for selected channels"]')
  )
  const input = required(form.querySelector('input'))
  const setter = Object.getOwnPropertyDescriptor(
    dom.HTMLInputElement.prototype,
    'value'
  )?.set
  assert.ok(setter)
  await act(async () => {
    setter.call(input, 'Primary')
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
  await act(async () => button(form, 'Apply').click())
  await until(
    () =>
      writes.length === 1 &&
      !container.querySelector(
        'form[aria-label="Set tag for selected channels"]'
      )
  )
  assert.deepEqual(writes[0], {
    ids: [1, 2],
    tag: 'Primary',
    expected_version: { routing: 1, pricing: 1 },
  })
  const first = container.querySelectorAll('[role=checkbox]')[1]
  await act(async () => (first as HTMLButtonElement).click())
  await until(() => container.textContent?.includes('1 selected') === true)
  batch = required(container.querySelector('[data-slot=channel-batch-actions]'))
  await act(async () => button(batch, 'Tag').click())
  form = required(
    container.querySelector('form[aria-label="Set tag for selected channels"]')
  )
  const draft = required(form.querySelector('input'))
  await act(async () => {
    setter.call(draft, 'Operations')
    draft.dispatchEvent(new Event('input', { bubbles: true }))
  })
  await act(async () => button(form, 'Apply').click())
  await until(
    () =>
      writes.length === 2 &&
      container.textContent?.includes('Tag conflict') === true
  )
  assert.equal(draft.value, 'Operations')
  assert.ok(container.querySelector('[data-slot=channel-batch-actions]'))
  await act(async () => button(form, 'Clear').click())
  await until(
    () =>
      writes.length === 3 &&
      !container.querySelector(
        'form[aria-label="Set tag for selected channels"]'
      )
  )
  assert.deepEqual(writes[2], {
    ids: [1],
    tag: null,
    expected_version: { routing: 2, pricing: 1 },
  })
  assert.equal(config.bindings.tags[1], undefined)
  assert.equal(config.bindings.tags[2], 'Primary')
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
  assert.equal(
    dialog.querySelector('[role=checkbox]')?.getAttribute('aria-checked'),
    'true'
  )
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

function required<T>(value: T | null | undefined): T {
  assert.ok(value)
  return value
}

const usageFixture = {
  scope: 'native_channel_window',
  start_timestamp: 1_700_000_000,
  end_timestamp: 1_700_000_000 + 31 * 86400,
  gross_settled_credits: '9007199254740993',
  refunded_credits: '2',
  net_used_credits: '9007199254740991',
  settled_requests: '9007199254740993',
  pending_potential_requests: '4',
  unknown_provenance_requests: '5',
  credits_per_usd: '500000',
  historical_coverage: false,
}

test('native channel card opens confirmed consumption with exact values and explicit coverage', async () => {
  const en = await import('@/i18n/locales/en.json')
  i18n.addResourceBundle('en', 'translation', en.default.translation)
  const reads: string[] = []
  api.request = (async (options: { url: string; method: string }) => {
    reads.push(options.url)
    assert.equal(options.method, 'GET')
    return {
      data: {
        success: true,
        data: options.url.endsWith('/usage') ? usageFixture : fixture(),
      },
    }
  }) as typeof api.request
  const container = await mount('channels', false)
  await until(() => !!container.textContent?.includes('Private provider'))
  assert.equal(button(container, 'Consumption').disabled, false)
  await act(async () => button(container, 'Consumption').click())
  await until(() => !!document.querySelector('dl'))
  const dialog = required(document.querySelector('[role=dialog]'))
  const text = dialog.textContent ?? ''
  assert.ok(text.includes('Confirmed native consumption'))
  assert.ok(
    text.includes(
      'Historical consumption and provider balances are not included.'
    )
  )
  assert.ok(text.includes('9,007,199,254,740,993'))
  assert.ok(text.includes('Unsettled potential requests'))
  assert.ok(text.includes('Requests without attribution evidence'))
  assert.equal(reads.filter((path) => path.endsWith('/usage')).length, 1)
  await act(async () => button(dialog, 'Close').click())
  await until(() => !document.querySelector('[role=dialog]'))
})

test('native usage errors and reused channel identities never turn into zero consumption', async () => {
  const { AxiosError, AxiosHeaders } = await import('axios')
  for (const failure of [403, 409, 501, 'malformed'] as const) {
    api.request = (async (options: { url: string }) => {
      if (!options.url.endsWith('/usage')) {
        return { data: { success: true, data: fixture() } }
      }
      if (failure === 'malformed') {
        return {
          data: {
            success: true,
            data: { ...usageFixture, net_used_credits: null },
          },
        }
      }
      throw new AxiosError(
        'internal implementation detail',
        'ERR_BAD_RESPONSE',
        undefined,
        undefined,
        {
          status: failure,
          statusText: 'unavailable',
          headers: {},
          config: { headers: new AxiosHeaders() },
          data: {},
        }
      )
    }) as typeof api.request
    const container = await mount('channels')
    await until(() => !!container.textContent?.includes('Private provider'))
    await act(async () => button(container, 'Consumption').click())
    await until(() => !!document.querySelector('[role=dialog] [role=alert]'))
    const dialog = required(document.querySelector('[role=dialog]'))
    const text = dialog.textContent ?? ''
    assert.equal(dialog.querySelector('dl'), null)
    assert.equal(text.includes('internal implementation detail'), false)
    if (failure === 409) {
      assert.match(text, /ID was reused.*historical identity/)
    }
    if (failure === 403) assert.match(text, /do not have permission/)
    await act(async () => button(dialog, 'Close').click())
    await dispose?.()
    dispose = undefined
  }
})

test('closing usage or switching sessions aborts the read and discards delayed results', async () => {
  let finish: (() => void) | undefined
  const signals: Array<AbortSignal | undefined> = []
  api.request = (async (options: { url: string; signal?: AbortSignal }) => {
    if (!options.url.endsWith('/usage')) {
      return { data: { success: true, data: fixture() } }
    }
    signals.push(options.signal)
    return new Promise<SuccessfulResponse<typeof usageFixture>>((resolve) => {
      finish = () => resolve({ data: { success: true, data: usageFixture } })
    })
  }) as typeof api.request
  const container = await mount('channels')
  await until(() => !!container.textContent?.includes('Private provider'))
  await act(async () => button(container, 'Consumption').click())
  await until(() => signals.length === 1)
  await act(async () =>
    button(required(document.querySelector('[role=dialog]')), 'Close').click()
  )
  assert.equal(signals[0]?.aborted, true)
  await act(async () => finish?.())
  assert.equal(document.querySelector('dl'), null)
  await act(async () => button(container, 'Consumption').click())
  await until(() => signals.length === 2)
  await act(async () => {
    login('replacement-usage-session', 10)
    await new Promise((resolve) => setTimeout(resolve, 20))
  })
  assert.equal(signals[1]?.aborted, true)
  await act(async () => finish?.())
  assert.equal(document.querySelector('[role=dialog]'), null)
  assert.equal(container.textContent?.includes('9,007,199,254,740,993'), false)
})

test('a confirmed zero native window displays zero instead of an unavailable state', async () => {
  const zero = {
    ...usageFixture,
    gross_settled_credits: '0',
    refunded_credits: '0',
    net_used_credits: '0',
    settled_requests: '0',
    pending_potential_requests: '0',
    unknown_provenance_requests: '0',
  }
  api.request = (async (options: { url: string }) => ({
    data: {
      success: true,
      data: options.url.endsWith('/usage') ? zero : fixture(),
    },
  })) as typeof api.request
  const container = await mount('channels')
  await until(() => !!container.textContent?.includes('Private provider'))
  await act(async () => button(container, 'Consumption').click())
  await until(() => document.querySelectorAll('dd').length === 6)
  for (const value of document.querySelectorAll('dd')) {
    assert.match(value.textContent ?? '', /^0(?: Credits)?$/)
  }
  assert.equal(document.querySelector('[role=dialog] [role=alert]'), null)
})

const balanceFixture = {
  balance: '9007199254740993.123456789',
  unit_label: 'provider points',
  balance_updated_time: null,
  kind: 'copied',
  recorded_at: 1_700_000_000,
  source_channel_id: 9,
}

test('provider balance is separate from confirmed consumption and preserves unknown, copied and reset observations', async () => {
  for (const observation of [
    balanceFixture,
    {
      ...balanceFixture,
      balance: null,
      unit_label: null,
      kind: 'unknown',
      source_channel_id: null,
      recorded_at: null,
    },
    { ...balanceFixture, balance: '0', kind: 'reset' },
  ]) {
    api.request = (async (options: { url: string }) => ({
      data: {
        success: true,
        data: options.url.endsWith('/counters')
          ? { ...countersFixture, balance: observation }
          : fixture(),
      },
    })) as typeof api.request
    const container = await mount('channels')
    await until(() => !!container.textContent?.includes('Private provider'))
    await act(async () => button(container, 'Provider balance').click())
    await until(() => !!document.querySelector('[role=dialog] dl'))
    const dialog = required(document.querySelector('[role=dialog]'))
    const displayed = required(dialog.querySelector('.text-2xl')).textContent
    assert.equal(
      displayed,
      observation.balance === null
        ? '-'
        : `${observation.balance} provider points`
    )
    assert.match(dialog.textContent ?? '', /Last updated:\s*-/)
    assert.equal(
      dialog.textContent?.includes('Confirmed native consumption'),
      false
    )
    if (observation.kind === 'unknown') {
      assert.ok(dialog.textContent?.includes('Unit unknown'))
    }
    if (observation.kind === 'copied') {
      assert.ok(dialog.textContent?.includes('Copied observation'))
    }
    if (observation.kind === 'reset') {
      assert.ok(dialog.textContent?.includes('Reset during copy'))
    }
    await act(async () => button(dialog, 'Close').click())
    await dispose?.()
    dispose = undefined
  }
})

test('a missing probe is disabled and failed explicit refreshes retain unknown rather than inventing a number', async () => {
  const { AxiosError, AxiosHeaders } = await import('axios')
  for (const status of [501, 403]) {
    api.request = (async (options: { url: string }) => {
      if (options.url.endsWith('/counters')) {
        return { data: { success: true, data: countersFixture } }
      }
      if (!options.url.endsWith('/balance')) {
        return {
          data: {
            success: true,
            data: {
              ...fixture(),
              bindings: {
                ...fixture().bindings,
                balance_probes: {
                  1: {
                    path: '/balance',
                    json_pointer: '/balance',
                    unit_label: null,
                  },
                },
              },
            },
          },
        }
      }
      throw new AxiosError(
        'private internal details',
        'ERR_BAD_RESPONSE',
        undefined,
        undefined,
        {
          status,
          statusText: 'unavailable',
          headers: {},
          config: { headers: new AxiosHeaders() },
          data: {},
        }
      )
    }) as typeof api.request
    const container = await mount('channels')
    await until(() => !!container.textContent?.includes('Private provider'))
    await act(async () => button(container, 'Provider balance').click())
    await until(() => !!document.querySelector('[role=dialog] dl'))
    const dialog = required(document.querySelector('[role=dialog]'))
    await act(async () => button(dialog, 'Update Balance').click())
    await until(() => !!dialog.querySelector('[role=alert]'))
    assert.equal(dialog.querySelector('.text-2xl')?.textContent, '-')
    assert.equal(
      dialog.textContent?.includes('private internal details'),
      false
    )
    if (status === 501) {
      assert.match(dialog.textContent ?? '', /Balance queries are disabled/)
      assert.equal(button(dialog, 'Update Balance').disabled, true)
    }
    await act(async () => button(dialog, 'Close').click())
    await dispose?.()
    dispose = undefined
  }
})

async function changeInput(input: HTMLInputElement, value: string) {
  await act(async () => {
    required(
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set
    ).call(input, value)
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
}

test('balance configuration preloads published fields, rejects invalid paths and sends one CAS save or explicit disable', async () => {
  let config: NativeConfiguration = {
    ...fixture(),
    bindings: {
      ...fixture().bindings,
      balance_probes: {
        1: {
          path: '/account/balance',
          json_pointer: '/available',
          unit_label: 'points',
        },
      },
    },
  }
  const writes: unknown[] = []
  api.request = (async (options: {
    url: string
    method: string
    data?: unknown
  }) => {
    if (options.method === 'GET') {
      return { data: { success: true, data: config } }
    }
    writes.push(options.data)
    assert.equal(
      options.url,
      '/core/v1/admin/routing/channels/1/balance-config'
    )
    config = {
      ...config,
      version: { ...config.version, routing: config.version.routing + 1 },
      routing: { ...config.routing, version: config.routing.version + 1 },
    }
    return { data: { success: true, data: { version: config.version } } }
  }) as typeof api.request
  const container = await mount('channels')
  await until(() => !!container.textContent?.includes('Private provider'))
  await act(async () => button(container, 'Configure balance query').click())
  let dialog = required(document.querySelector('[role=dialog]'))
  const inputs = [...dialog.querySelectorAll('input')]
  assert.deepEqual(
    inputs.map((input) => input.value),
    ['/account/balance', '/available', 'points']
  )
  await changeInput(required(inputs[0]), '//elsewhere.test/balance')
  await act(async () => button(dialog, 'Save').click())
  assert.equal(writes.length, 0)
  assert.ok(dialog.querySelector('[role=alert]'))
  await changeInput(required(inputs[0]), '/balance')
  await changeInput(required(inputs[1]), '/a~1b')
  await changeInput(required(inputs[2]), '')
  await act(async () => {
    button(dialog, 'Save').click()
    button(dialog, 'Save').click()
  })
  await until(() => !document.querySelector('[role=dialog]'))
  assert.deepEqual(writes, [
    {
      expected_version: { routing: 1, pricing: 1 },
      probe: { path: '/balance', json_pointer: '/a~1b', unit_label: null },
    },
  ])
  await act(async () => button(container, 'Configure balance query').click())
  dialog = required(document.querySelector('[role=dialog]'))
  await act(async () => button(dialog, 'Disable').click())
  await until(() => !document.querySelector('[role=dialog]'))
  assert.deepEqual(writes[1], {
    expected_version: { routing: 2, pricing: 1 },
    probe: null,
  })
})

test('Native copy sends false only after explicitly preserving balance and display baselines', async () => {
  let copied: unknown
  api.request = (async (options: { method: string; data?: unknown }) => {
    if (options.method === 'GET') {
      return { data: { success: true, data: fixture() } }
    }
    copied = options.data
    return {
      data: {
        success: true,
        data: { id: 2, version: { routing: 2, pricing: 1 } },
      },
    }
  }) as typeof api.request
  const container = await mount('channels')
  const dialog = await openCopy(container)
  const checkbox = required(
    dialog.querySelector<HTMLElement>('[role=checkbox]')
  )
  await act(async () => checkbox.click())
  assert.equal(checkbox.getAttribute('aria-checked'), 'false')
  assert.ok(
    dialog.textContent?.includes('unknown historical coverage stays unknown')
  )
  await act(async () => button(dialog, 'Copy Channel').click())
  await until(() => !document.querySelector('[role=dialog]'))
  assert.deepEqual(copied, {
    expected_version: { routing: 1, pricing: 1 },
    suffix: '_copy',
    reset_balance: false,
  })
})

const countersFixture = {
  balance: {
    balance: null,
    unit_label: null,
    balance_updated_time: null,
    kind: 'unknown',
    recorded_at: null,
    source_channel_id: null,
  },
  native: {
    ...usageFixture,
    scope: 'native_channel_totals',
    start_timestamp: 0,
    gross_settled_credits: '20',
    refunded_credits: '2',
    net_used_credits: '18',
  },
  inherited: {
    gross_settled_credits: '5',
    refunded_credits: '1',
    net_used_credits: '4',
  },
  combined: {
    gross_settled_credits: '25',
    refunded_credits: '3',
    net_used_credits: '22',
  },
  historical_coverage: false,
  complete: false,
  used_quota: null,
  credits_per_usd: '500000',
}

test('display subtotals keep copied inheritance separate from native consumption and incomplete history', async () => {
  api.request = (async (options: { url: string }) => ({
    data: {
      success: true,
      data: options.url.endsWith('/usage')
        ? usageFixture
        : options.url.endsWith('/counters')
          ? countersFixture
          : fixture(),
    },
  })) as typeof api.request
  const container = await mount('channels')
  await until(() => !!container.textContent?.includes('Private provider'))
  await act(async () => button(container, 'Consumption').click())
  await until(
    () =>
      document.querySelectorAll('[aria-label="Display usage"] dd').length === 3
  )
  const section = required(
    document.querySelector('[aria-label="Display usage"]')
  )
  assert.deepEqual(
    [...section.querySelectorAll('dd')].map((value) => value.textContent),
    ['18 Credits', '4 Credits', '22 Credits']
  )
  assert.match(section.textContent ?? '', /not a complete cumulative total/)
  assert.match(section.textContent ?? '', /not new consumption/)
})

test('reviewed legacy lifetime keeps the original unit and an exact repeating amount alongside native credits', async () => {
  const counters = {
    ...countersFixture,
    native: { ...countersFixture.native, unknown_provenance_requests: '0' },
    lifetime: {
      basis: 'legacy_recorded_plus_native',
      complete: true,
      total_usd: '0.333377333333333333',
      exact_usd_numerator: '500066',
      exact_usd_denominator: '1500000',
      legacy_opening: {
        kind: 'imported',
        source_channel_id: '9',
        raw_used_quota: '1',
        credits_per_usd: '3',
        recorded_usd: '0.333333333333333333',
        observed_at: null,
        archive_sha256: 'a'.repeat(64),
      },
    },
  }
  Object.defineProperty(api, 'request', {
    configurable: true,
    writable: true,
    value: async (options: { url: string }) => ({
      data: {
        success: true,
        data: options.url.endsWith('/counters')
          ? counters
          : options.url.endsWith('/usage')
            ? usageFixture
            : fixture(),
      },
    }),
  })
  const container = await mount('channels')
  await until(() => !!container.textContent?.includes('Private provider'))
  await act(async () => button(container, 'Consumption').click())
  await until(
    () =>
      document.querySelectorAll('[aria-label="Display usage"] dd').length === 5
  )
  const section = required(
    document.querySelector('[aria-label="Display usage"]')
  )
  assert.deepEqual(
    [...section.querySelectorAll('dd')].map((value) => value.textContent),
    [
      '0.333377333333333333 USD',
      '0.333333333333333333 USD',
      '18 Credits',
      '4 Credits',
      '22 Credits',
    ]
  )
  assert.match(
    section.textContent ?? '',
    /Source channel 9 · 1 units \/ 3 per USD/
  )
  assert.match(
    section.textContent ?? '',
    /do not provide a gross\/refund breakdown/
  )
  assert.equal(
    section.textContent?.includes('not a complete cumulative total'),
    false
  )
})

test('an absent inherited display baseline remains unknown rather than displaying zero', async () => {
  const absent = {
    ...countersFixture,
    inherited: null,
    combined: {
      gross_settled_credits: countersFixture.native.gross_settled_credits,
      refunded_credits: countersFixture.native.refunded_credits,
      net_used_credits: countersFixture.native.net_used_credits,
    },
  }
  api.request = (async (options: { url: string }) => ({
    data: {
      success: true,
      data: options.url.endsWith('/usage')
        ? usageFixture
        : options.url.endsWith('/counters')
          ? absent
          : fixture(),
    },
  })) as typeof api.request
  const container = await mount('channels')
  await until(() => !!container.textContent?.includes('Private provider'))
  await act(async () => button(container, 'Consumption').click())
  await until(
    () =>
      document.querySelectorAll('[aria-label="Display usage"] dd').length === 3
  )
  const values = [
    ...document.querySelectorAll('[aria-label="Display usage"] dd'),
  ].map((element) => element.textContent)
  assert.deepEqual(values, ['18 Credits', '-', '18 Credits'])
})

test('closing a stored balance read aborts it and session changes discard a delayed configuration save', async () => {
  let finishRead: (() => void) | undefined
  let finishSave: (() => void) | undefined
  let readSignal: AbortSignal | undefined
  let writes = 0
  api.request = (async (options: {
    url: string
    method: string
    signal?: AbortSignal
  }) => {
    if (options.url.endsWith('/counters')) {
      readSignal = options.signal
      return new Promise<SuccessfulResponse<typeof countersFixture>>(
        (resolve) => {
          finishRead = () =>
            resolve({ data: { success: true, data: countersFixture } })
        }
      )
    }
    if (options.method === 'PUT') {
      writes++
      return new Promise<
        SuccessfulResponse<{ version: NativeConfiguration['version'] }>
      >((resolve) => {
        finishSave = () =>
          resolve({
            data: {
              success: true,
              data: { version: { routing: 2, pricing: 1 } },
            },
          })
      })
    }
    return { data: { success: true, data: fixture() } }
  }) as typeof api.request
  const container = await mount('channels')
  await until(() => !!container.textContent?.includes('Private provider'))
  await act(async () => button(container, 'Provider balance').click())
  await until(() => !!readSignal)
  await act(async () =>
    button(required(document.querySelector('[role=dialog]')), 'Close').click()
  )
  assert.equal(readSignal?.aborted, true)
  await act(async () => finishRead?.())
  assert.equal(document.querySelector('[role=dialog]'), null)
  await act(async () => button(container, 'Configure balance query').click())
  const dialog = required(document.querySelector('[role=dialog]'))
  await act(async () => button(dialog, 'Disable').click())
  assert.equal(writes, 1)
  await act(async () => {
    login('replacement-balance-session', 10)
    await new Promise((resolve) => setTimeout(resolve, 20))
  })
  await act(async () => finishSave?.())
  assert.equal(document.querySelector('[role=dialog]'), null)
  assert.equal(
    container.textContent?.includes('The save result could not be confirmed'),
    false
  )
  assert.equal(writes, 1)
})

test('old managed Core keeps default copy compatible and blocks accounting controls and preservation', async () => {
  let writes = 0
  api.request = (async (options: {
    method: string
    url: string
    data?: unknown
  }) => {
    if (options.method === 'GET') {
      assert.equal(options.url, '/core/v1/admin/routing/configuration')
      return { data: { success: true, data: fixture() } }
    }
    writes++
    assert.equal(options.url, '/core/v1/admin/routing/channels/1/copy')
    assert.deepEqual(options.data, {
      expected_version: { routing: 1, pricing: 1 },
      suffix: '_copy',
    })
    return {
      data: {
        success: true,
        data: { id: 9, version: { routing: 2, pricing: 1 } },
      },
    }
  }) as typeof api.request
  const container = await mount('channels', true, {})
  await until(() => !!container.textContent?.includes('Private provider'))
  for (const key of [
    'channelUsage.open',
    'channelBalance.title',
    'channelBalance.configure',
  ]) {
    assert.equal(
      [...container.querySelectorAll('button')].some(
        (item) => item.textContent === i18n.t(key)
      ),
      false
    )
  }
  const dialog = await openCopy(container)
  const reset = required(dialog.querySelector('[role=checkbox]'))
  assert.ok(reset instanceof HTMLElement)
  assert.ok(reset.matches(':disabled, [aria-disabled="true"]'))
  await act(async () => reset.click())
  assert.equal(reset.getAttribute('aria-checked'), 'true')
  assert.equal(writes, 0)
  assert.equal(button(dialog, 'Copy Channel').disabled, false)
  await act(async () => button(dialog, 'Copy Channel').click())
  await until(() => !document.querySelector('[role=dialog]'))
  assert.equal(writes, 1)
})

test('accounting capabilities gate window reads, display counters and provider refresh independently', async () => {
  for (const windowEnabled of [true, false]) {
    const reads: string[] = []
    const config = fixture()
    api.request = (async (options: { url: string }) => {
      reads.push(options.url)
      return {
        data: {
          success: true,
          data: options.url.endsWith('/usage')
            ? usageFixture
            : options.url.endsWith('/counters')
              ? countersFixture
              : {
                  ...config,
                  bindings: {
                    ...config.bindings,
                    balance_probes: {
                      1: {
                        path: '/balance',
                        json_pointer: '/balance',
                        unit_label: null,
                      },
                    },
                  },
                },
        },
      }
    }) as typeof api.request
    const container = await mount('channels', true, {
      native_channel_usage: windowEnabled,
      native_channel_counters: !windowEnabled,
      native_balance_probe: false,
      copy_preserve_counters: false,
    })
    await until(() => !!container.textContent?.includes('Private provider'))
    const labels = new Set(
      [...container.querySelectorAll('button')].map((item) => item.textContent)
    )
    assert.equal(labels.has(i18n.t('channelUsage.open')), windowEnabled)
    assert.equal(labels.has(i18n.t('channelBalance.title')), !windowEnabled)
    assert.equal(labels.has(i18n.t('channelBalance.configure')), false)
    await act(async () =>
      button(
        container,
        i18n.t(windowEnabled ? 'channelUsage.open' : 'channelBalance.title')
      ).click()
    )
    await until(() => !!document.querySelector('dl'))
    const dialog = required(document.querySelector('[role=dialog]'))
    if (windowEnabled) {
      assert.equal(
        reads.some((url) => url.endsWith('/counters')),
        false
      )
      assert.equal(
        [...dialog.querySelectorAll('section')].some(
          (item) =>
            item.getAttribute('aria-label') === i18n.t('channelCounters.title')
        ),
        false
      )
    } else {
      assert.equal(
        reads.some((url) => url.endsWith('/usage')),
        false
      )
      assert.equal(button(dialog, 'Update Balance').disabled, true)
      await act(async () => button(dialog, 'Update Balance').click())
      assert.equal(
        reads.some((url) => url.endsWith('/balance')),
        false
      )
    }
    await dispose?.()
    dispose = undefined
  }
})

test('expired capabilities cannot submit a previously selected preserve copy or retain new API entry points', async () => {
  let writes = 0
  api.request = (async (options: { method: string }) => {
    if (options.method !== 'GET') writes++
    return { data: { success: true, data: fixture() } }
  }) as typeof api.request
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const container = await mount(
    'channels',
    true,
    accountingCapabilities,
    client
  )
  const dialog = await openCopy(container)
  const reset = required(dialog.querySelector('[role=checkbox]'))
  assert.ok(reset instanceof HTMLElement)
  await act(async () => reset.click())
  assert.equal(reset.getAttribute('aria-checked'), 'false')
  await act(async () => {
    client.setQueryData(
      ['channel-ops'],
      {
        success: true,
        data: { native_secrets: true, ...accountingCapabilities },
      },
      { updatedAt: Date.now() - 60_001 }
    )
  })
  await until(() => button(dialog, 'Copy Channel').disabled)
  assert.equal(button(dialog, 'Copy Channel').disabled, true)
  for (const key of [
    'channelUsage.open',
    'channelBalance.title',
    'channelBalance.configure',
  ]) {
    assert.equal(
      [...container.querySelectorAll('button')].some(
        (item) => item.textContent === i18n.t(key)
      ),
      false
    )
  }
  await act(async () => button(dialog, 'Copy Channel').click())
  assert.equal(writes, 0)
})

test('channel creation uses the permanent ID high water without publishing the read hint and keeps the old Core fallback', async () => {
  for (const highWater of [17, 0, undefined]) {
    const expectedId = highWater === 17 ? 18 : highWater === 0 ? 1 : 2
    const current = fixture()
    const config: NativeConfiguration = {
      ...current,
      ...(highWater === undefined ? {} : { channel_id_high_water: highWater }),
      routing: {
        ...current.routing,
        upstreams: highWater === 0 ? [] : current.routing.upstreams,
        routes: highWater === 0 ? [] : current.routing.routes,
      },
      bindings: {
        ...current.bindings,
        names: highWater === 0 ? {} : current.bindings.names,
      },
    }
    let writes = 0
    api.request = (async (options: {
      method: string
      url: string
      data?: unknown
    }) => {
      if (options.method === 'GET') {
        return { data: { success: true, data: config } }
      }
      writes++
      assert.equal(options.method, 'POST')
      assert.equal(options.url, '/core/v1/admin/routing/channels')
      assert.deepEqual(options.data, {
        expected_version: { routing: 1, pricing: 1 },
        id: expectedId,
        name: 'New provider',
        tag: 'Launch',
        endpoint: 'https://new-provider.example/v1/',
        protocol: 'openai_chat',
        enabled: false,
        contexts: [],
        attachments: [],
        secret: 'synthetic-create-key',
      })
      return {
        data: { success: true, data: { version: { routing: 2, pricing: 1 } } },
      }
    }) as typeof api.request
    const container = await mount('channels')
    await until(() =>
      [...container.querySelectorAll('button')].some(
        (item) => item.textContent === 'Create Channel'
      )
    )
    await act(async () => button(container, 'Create Channel').click())
    const textarea = required(container.querySelector('textarea'))
    const draft = parseNativeDocument(textarea.value)
    assert.equal(draft.id, expectedId)
    assert.equal(Object.hasOwn(draft, 'channel_id_high_water'), false)
    assert.equal(draft.tag, '')
    draft.name = 'New provider'
    draft.tag = 'Launch'
    draft.endpoint = 'https://new-provider.example/v1/'
    const textareaSetter = required(
      Object.getOwnPropertyDescriptor(
        dom.HTMLTextAreaElement.prototype,
        'value'
      )?.set
    )
    const secret = required(container.querySelector('input[type=password]'))
    const secretSetter = required(
      Object.getOwnPropertyDescriptor(dom.HTMLInputElement.prototype, 'value')
        ?.set
    )
    await act(async () => {
      textareaSetter.call(textarea, JSON.stringify(draft))
      textarea.dispatchEvent(new Event('input', { bubbles: true }))
      secretSetter.call(secret, 'synthetic-create-key')
      secret.dispatchEvent(new Event('input', { bubbles: true }))
    })
    await act(async () => button(container, 'Save').click())
    await until(() => writes === 1 && !container.querySelector('textarea'))
    assert.equal(writes, 1)
    await dispose?.()
    dispose = undefined
  }
})

test('native key manager enables a pooled key and changes polling with release CAS', async () => {
  let config = fixture()
  let version = 1
  let secondaryEnabled = false
  let polling = true
  const writes: Array<Record<string, unknown>> = []
  api.request = (async (options: {
    method: string
    url: string
    data?: Record<string, unknown>
  }) => {
    if (options.method === 'GET') {
      if (options.url.endsWith('/keys')) {
        return {
          data: {
            success: true,
            data: {
              version: { routing: version, pricing: 1 },
              active_reference: { id: 7, revision: 1 },
              polling,
              plaintext_available: false,
              keys: [
                {
                  reference: { id: 7, revision: 1 },
                  label: 'Primary',
                  created_at: '2026-11-12T00:00:00Z',
                  enabled: true,
                },
                {
                  reference: { id: 8, revision: 1 },
                  label: 'Secondary',
                  created_at: '2026-11-12T00:00:00Z',
                  enabled: secondaryEnabled,
                },
              ],
            },
          },
        }
      }
      return { data: { success: true, data: config } }
    }
    assert.equal(options.url, '/core/v1/admin/routing/channels/1/keys')
    assert.ok(options.data)
    writes.push(options.data)
    if (options.data.operation === 'set_enabled') {
      secondaryEnabled = true
    } else {
      assert.equal(options.data.operation, 'set_polling')
      polling = false
    }
    version += 1
    config = {
      ...config,
      version: { routing: version, pricing: 1 },
      routing: { ...config.routing, version },
    }
    return {
      data: {
        success: true,
        data: {
          version: { routing: version, pricing: 1 },
          reference: options.data.reference ?? { id: 7, revision: 1 },
        },
      },
    }
  }) as typeof api.request
  const container = await mount('channels')
  await until(() =>
    [...container.querySelectorAll('button')].some(
      (item) => item.textContent === 'API Keys'
    )
  )
  await act(async () => button(container, 'API Keys').click())
  await until(() => !!document.querySelector('[role=dialog]'))
  const dialog = required(document.querySelector('[role=dialog]'))
  await until(() => !!dialog.textContent?.includes('Secondary'))
  assert.equal(button(dialog, 'Disable').disabled, true)
  await act(async () => {
    button(dialog, 'Enable').click()
    button(dialog, 'Enable').click()
  })
  await until(() => writes.length === 1 && !button(dialog, 'Add').disabled)
  assert.deepEqual(writes[0], {
    expected_version: { routing: 1, pricing: 1 },
    operation: 'set_enabled',
    reference: { id: 8, revision: 1 },
    enabled: true,
  })
  const pollingToggle = required(dialog.querySelector('[role=checkbox]'))
  await act(async () => (pollingToggle as HTMLElement).click())
  await until(() => writes.length === 2)
  assert.deepEqual(writes[1], {
    expected_version: { routing: 2, pricing: 1 },
    operation: 'set_polling',
    enabled: false,
  })
})

test('channel editing exposes common fields and preserves bindings, context metadata and the existing key', async () => {
  const config: NativeConfiguration = fixture()
  config.bindings.contexts = [
    {
      upstream_id: 1,
      upstream_model: 'model-a',
      protocol: 'openai_chat',
      endpoint: 'https://private-provider.example/v1/',
      max_input_tokens: 8192,
      max_output_tokens: 4096,
      contract_revision: 'synthetic-preserved-context',
    },
  ]
  const writes: Record<string, unknown>[] = []
  api.request = (async (options: {
    method: string
    url: string
    data?: Record<string, unknown>
  }) => {
    if (options.method === 'GET') {
      return { data: { success: true, data: config } }
    }
    assert.equal(options.method, 'PUT')
    assert.equal(options.url, '/core/v1/admin/routing/channels/1')
    assert.ok(options.data)
    writes.push(options.data)
    return {
      data: { success: true, data: { version: { routing: 2, pricing: 1 } } },
    }
  }) as typeof api.request
  const container = await mount('channels')
  await until(() =>
    [...container.querySelectorAll('button')].some(
      (item) => item.textContent === 'Edit'
    )
  )
  await act(async () => button(container, 'Edit').click())
  const form = required(
    container.querySelector('form[aria-label="Edit Channel"]')
  )
  assert.equal(container.querySelector('input[type=password]'), null)
  assert.equal(required(form.querySelector('details')).open, false)
  const setter = required(
    Object.getOwnPropertyDescriptor(dom.HTMLInputElement.prototype, 'value')
      ?.set
  )
  function field(label: string) {
    const element = [...form.querySelectorAll('label')].find(
      (item) => item.querySelector('span')?.textContent === label
    )
    return required(element?.querySelector('input'))
  }
  assert.equal(field('Name').value, 'Private provider')
  await act(async () => {
    setter.call(field('Name'), 'Edited provider')
    field('Name').dispatchEvent(new Event('input', { bubbles: true }))
  })
  await act(async () => {
    setter.call(field('Base URL'), 'https://edited-provider.example/v1/')
    field('Base URL').dispatchEvent(new Event('input', { bubbles: true }))
  })
  const protocol = required(
    [...form.querySelectorAll('label')]
      .find((item) => item.querySelector('span')?.textContent === 'Protocol')
      ?.querySelector('select')
  )
  assert.equal(protocol.value, 'openai_chat')
  assert.ok(protocol.textContent?.includes('OpenAI Responses'))
  await act(async () => {
    protocol.value = 'openai_responses'
    protocol.dispatchEvent(new Event('change', { bubbles: true }))
  })
  await act(async () => button(form, 'Save').click())
  await until(() => writes.length === 1)
  assert.equal(writes[0].name, 'Edited provider')
  assert.equal(writes[0].protocol, 'openai_responses')
  assert.equal(Object.hasOwn(writes[0], 'secret'), false)
  assert.deepEqual(writes[0].expected_version, { routing: 1, pricing: 1 })
  assert.deepEqual(writes[0].contexts, [
    {
      ...config.bindings.contexts[0],
      endpoint: 'https://edited-provider.example/v1/',
      protocol: 'openai_responses',
    },
  ])
  assert.deepEqual(writes[0].attachments, [
    {
      route_id: 1,
      upstream_model: 'model-a',
      priority: 0,
      weight: 1,
      speeds: ['standard'],
      payload: 'adapt',
      timeout_ms: 30000,
    },
  ])
})

test('channel bindings add, map and remove models through one reviewed channel draft', async () => {
  const config: NativeConfiguration = fixture()
  config.routing.groups.push({ name: 'team', enabled: true })
  config.routing.routes.push({
    ...config.routing.routes[0],
    id: 2,
    model: 'model-b',
    group: 'team',
    targets: [],
  })
  config.prices.rules.push({
    ...config.prices.rules[0],
    id: 2,
    model: 'model-b',
    group: 'team',
  })
  config.bindings.contexts = [
    {
      upstream_id: 1,
      upstream_model: 'model-a',
      protocol: 'openai_chat',
      endpoint: config.routing.upstreams[0].endpoint,
      max_input_tokens: 8192,
      max_output_tokens: 4096,
      contract_revision: 'supplier-a',
      reservation: { mode: 'legacy_upper_bound' },
    },
  ]
  const writes: Record<string, unknown>[] = []
  api.request = (async (options: {
    method: string
    url: string
    data?: Record<string, unknown>
  }) => {
    if (options.method === 'GET') {
      return { data: { success: true, data: config } }
    }
    assert.equal(options.method, 'PUT')
    assert.equal(options.url, '/core/v1/admin/routing/channels/1')
    writes.push(required(options.data))
    return {
      data: { success: true, data: { version: { routing: 2, pricing: 1 } } },
    }
  }) as typeof api.request
  const container = await mount('channels')
  await until(() =>
    [...container.querySelectorAll('button')].some(
      (item) => item.textContent === 'Edit'
    )
  )
  await act(async () => button(container, 'Edit').click())
  const form = required(
    container.querySelector<HTMLFormElement>('form[aria-label="Edit Channel"]')
  )
  const textarea = required(form.querySelector('textarea'))
  const routeFields = (id: number) =>
    required(form.querySelector(`fieldset[data-route-id="${id}"]`))
  const field = (scope: Element, label: string) =>
    required(
      [...scope.querySelectorAll('label')]
        .find((item) => item.querySelector('span')?.textContent === label)
        ?.querySelector('input')
    )
  const picker = required(
    form.querySelector('section[aria-label="Available models"]')
  )
  const choice = required(
    [...picker.querySelectorAll('label')]
      .find((item) => item.textContent?.includes('model-b'))
      ?.querySelector<HTMLInputElement>('input[type=checkbox]')
  )
  await act(async () => choice.click())
  await act(async () => button(form, 'Add selected models').click())
  assert.ok(routeFields(2).textContent?.includes('model-b'))
  assert.ok(routeFields(2).closest('details')?.textContent?.includes('team'))
  assert.equal(required(routeFields(2).closest('details')).open, true)
  assert.equal(required(routeFields(1).closest('details')).open, false)
  assert.equal(writes.length, 0)
  assert.equal(form.checkValidity(), false)
  await changeInput(field(routeFields(2), 'Upstream model'), 'supplier-b')
  await changeInput(field(routeFields(2), 'Priority'), '2')
  await changeInput(field(routeFields(2), 'Weight'), '')
  assert.ok(routeFields(2))
  assert.equal(JSON.parse(textarea.value).attachments[1].weight, '')
  assert.equal(form.checkValidity(), false)
  await changeInput(field(routeFields(2), 'Weight'), '7')
  await changeInput(field(routeFields(2), 'Timeout (ms)'), '15000')
  await changeInput(field(routeFields(2), 'Maximum input tokens'), '16384')
  await changeInput(field(routeFields(2), 'Maximum output tokens'), '2048')
  await changeInput(field(routeFields(2), 'Contract revision'), 'supplier-b-v1')
  assert.equal(required(routeFields(2).querySelector('details')).open, true)
  await act(async () => button(routeFields(1), 'Remove').click())
  assert.equal(form.querySelector('[data-route-id="1"]'), null)
  assert.equal(writes.length, 0)
  const draft = JSON.parse(textarea.value)
  assert.deepEqual(draft.expected_version, { routing: 1, pricing: 1 })
  assert.deepEqual(draft.attachments, [
    {
      route_id: 2,
      upstream_model: 'supplier-b',
      priority: 2,
      weight: 7,
      speeds: ['standard'],
      timeout_ms: 15000,
      payload: 'adapt',
    },
  ])
  assert.deepEqual(draft.contexts, [
    {
      upstream_id: 1,
      upstream_model: 'supplier-b',
      endpoint: config.routing.upstreams[0].endpoint,
      protocol: 'openai_chat',
      max_input_tokens: 16384,
      max_output_tokens: 2048,
      contract_revision: 'supplier-b-v1',
    },
  ])
  assert.equal(form.checkValidity(), true)
  await act(async () => button(form, 'Save').click())
  await until(() => writes.length === 1)
  assert.deepEqual(writes[0], draft)
})

test('channel model multi-selection survives filtering and adds distinct bindings in one unchanged CAS draft', async () => {
  const config: NativeConfiguration = fixture()
  config.routing.groups.push({ name: 'team', enabled: true })
  for (const [id, model, group] of [
    [2, 'model-b', 'team'],
    [3, 'model-b', 'default'],
    [4, 'model-c', 'team'],
  ] as const) {
    config.routing.routes.push({
      ...config.routing.routes[0],
      id,
      model,
      group,
      targets: [],
    })
  }
  config.bindings.contexts = [
    {
      upstream_id: 1,
      upstream_model: 'model-a',
      protocol: 'openai_chat',
      endpoint: config.routing.upstreams[0].endpoint,
      max_input_tokens: 8192,
      max_output_tokens: 4096,
      contract_revision: 'supplier-a',
      reservation: { mode: 'legacy_upper_bound' },
    },
  ]
  api.request = (async (options: { method: string }) => {
    assert.equal(
      options.method,
      'GET',
      'model selection only edits the local draft'
    )
    return { data: { success: true, data: config } }
  }) as typeof api.request
  const container = await mount('channels')
  await until(() =>
    [...container.querySelectorAll('button')].some(
      (item) => item.textContent === 'Edit'
    )
  )
  await act(async () => button(container, 'Edit').click())
  const form = required(
    container.querySelector<HTMLFormElement>('form[aria-label="Edit Channel"]')
  )
  const picker = required(
    form.querySelector('section[aria-label="Available models"]')
  )
  const search = required(
    picker.querySelector<HTMLInputElement>(
      'input[aria-label="Search models or groups"]'
    )
  )
  const selectMatches = () =>
    required(
      [...picker.querySelectorAll('label')]
        .find((item) => item.textContent?.includes('Select all matches'))
        ?.querySelector<HTMLInputElement>('input[type=checkbox]')
    )
  await changeInput(search, 'model-b')
  await act(async () => selectMatches().click())
  assert.ok(picker.textContent?.includes('2 selected'))
  await changeInput(search, 'no matching model')
  assert.ok(picker.textContent?.includes('No results found'))
  assert.ok(picker.textContent?.includes('2 selected'))
  assert.equal(button(picker, 'Add selected models').disabled, false)
  await changeInput(search, 'team')
  await act(async () => selectMatches().click())
  assert.ok(picker.textContent?.includes('3 selected'))
  await act(async () => selectMatches().click())
  assert.ok(picker.textContent?.includes('1 selected'))
  await act(async () => selectMatches().click())
  await act(async () => button(picker, 'Add selected models').click())
  const textarea = required(form.querySelector('textarea'))
  const draft = JSON.parse(textarea.value)
  assert.deepEqual(draft.expected_version, { routing: 1, pricing: 1 })
  assert.deepEqual(
    draft.attachments.map((item: { route_id: number }) => item.route_id),
    [1, 2, 3, 4]
  )
  assert.deepEqual(draft.contexts[0], config.bindings.contexts[0])
  assert.deepEqual(
    draft.contexts.map(
      (item: { upstream_model: string }) => item.upstream_model
    ),
    ['model-a', 'model-b', 'model-c']
  )
  assert.equal(Object.hasOwn(draft, 'secret'), false)
  assert.equal(
    form.checkValidity(),
    false,
    'new supplier contracts still require explicit input'
  )
  for (const id of [2, 3, 4]) {
    const fields = required(
      form.querySelector(`fieldset[data-route-id="${id}"]`)
    )
    assert.equal(required(fields.closest('details')).open, true)
  }
  const details = required(
    form.querySelector('fieldset[data-route-id="2"]')?.closest('details')
  )
  const before = textarea.value
  await act(async () => {
    details.open = false
    details.dispatchEvent(new Event('toggle'))
  })
  assert.equal(textarea.value, before)
  await act(async () => {
    details.open = true
    details.dispatchEvent(new Event('toggle'))
  })
  assert.equal(textarea.value, before)
  assert.equal(
    form.querySelector('section[aria-label="Available models"]'),
    null
  )
})

test('the channel page exposes editing immediately without a configuration toggle or a duplicate read-only list', async () => {
  const config = fixture()
  api.request = (async (options: { method: string; url: string }) => {
    assert.equal(options.method, 'GET')
    assert.equal(options.url, '/core/v1/admin/routing/configuration')
    return { data: { success: true, data: config } }
  }) as typeof api.request
  const container = await mount(
    'channels',
    false,
    accountingCapabilities,
    undefined,
    true
  )
  await until(() =>
    [...container.querySelectorAll('button')].some(
      (item) => item.textContent === 'Edit'
    )
  )
  assert.equal(
    [...container.querySelectorAll('button')].some(
      (item) => item.textContent === 'Configuration'
    ),
    false
  )
  assert.equal(
    container.querySelectorAll('input:not([type=checkbox])').length,
    1
  )
  await act(async () => button(container, 'Edit').click())
  assert.ok(container.querySelector('form[aria-label="Edit Channel"]'))
  assert.equal(container.querySelector('input[type=password]'), null)
})

test('channel drafts survive current-section clicks and require confirmation before switching or replacing the editor', async () => {
  api.request = (async (options: { method: string }) => {
    assert.equal(options.method, 'GET', 'Discarding a draft must not write')
    return { data: { success: true, data: fixture() } }
  }) as typeof api.request
  const container = await mount('channels')
  await until(() => !!container.textContent?.includes('Private provider'))
  await act(async () => button(container, 'Edit').click())
  const name = required(
    container.querySelector('form input')
  ) as HTMLInputElement
  const setter = required(
    Object.getOwnPropertyDescriptor(dom.HTMLInputElement.prototype, 'value')
      ?.set
  )
  await act(async () => {
    setter.call(name, 'Unpublished provider')
    name.dispatchEvent(new Event('input', { bubbles: true }))
  })
  await act(async () => button(container, 'Channels').click())
  assert.equal(name.value, 'Unpublished provider')
  assert.equal(document.querySelector('[role=dialog]'), null)
  assert.equal(
    button(container, 'Channels').getAttribute('aria-current'),
    'page'
  )

  await act(async () => button(container, 'Models').click())
  let dialog = required(document.querySelector('[role=dialog]'))
  assert.match(dialog.textContent ?? '', /unsaved changes/)
  await act(async () => button(dialog, 'Stay').click())
  assert.equal(name.value, 'Unpublished provider')
  assert.equal(
    button(container, 'Channels').getAttribute('aria-current'),
    'page'
  )

  await act(async () => button(container, 'Create Channel').click())
  dialog = required(document.querySelector('[role=dialog]'))
  await act(async () => button(dialog, 'Leave').click())
  assert.ok(container.querySelector('form[aria-label="Create Channel"]'))
  assert.equal(
    (container.querySelector('form input') as HTMLInputElement).value,
    ''
  )
  // An untouched replacement may close without another confirmation.
  await act(async () =>
    button(required(container.querySelector('form')), 'Cancel').click()
  )
  assert.equal(container.querySelector('form'), null)
  assert.equal(document.querySelector('[role=dialog]'), null)
})

test('invalid advanced drafts and unsaved API keys are protected, and confirmed section changes clear secrets', async () => {
  api.request = (async (options: { method: string }) => {
    assert.equal(options.method, 'GET')
    return { data: { success: true, data: fixture() } }
  }) as typeof api.request
  const container = await mount('channels')
  await until(() => !!container.textContent?.includes('Private provider'))
  await act(async () => button(container, 'Edit').click())
  const textarea = required(container.querySelector('textarea'))
  const setText = required(
    Object.getOwnPropertyDescriptor(dom.HTMLTextAreaElement.prototype, 'value')
      ?.set
  )
  await act(async () => {
    setText.call(textarea, '{unfinished')
    textarea.dispatchEvent(new Event('input', { bubbles: true }))
  })
  await act(async () =>
    button(required(container.querySelector('form')), 'Cancel').click()
  )
  let dialog = required(document.querySelector('[role=dialog]'))
  await act(async () => button(dialog, 'Stay').click())
  assert.equal(textarea.value, '{unfinished')
  await act(async () =>
    button(required(container.querySelector('form')), 'Cancel').click()
  )
  dialog = required(document.querySelector('[role=dialog]'))
  await act(async () => button(dialog, 'Leave').click())
  assert.equal(container.querySelector('form'), null)

  await act(async () => button(container, 'Create Channel').click())
  const secret = required(
    container.querySelector('input[type=password]')
  ) as HTMLInputElement
  const setInput = required(
    Object.getOwnPropertyDescriptor(dom.HTMLInputElement.prototype, 'value')
      ?.set
  )
  await act(async () => {
    setInput.call(secret, 'synthetic-unsaved-secret')
    secret.dispatchEvent(new Event('input', { bubbles: true }))
  })
  await act(async () => button(container, 'Models').click())
  dialog = required(document.querySelector('[role=dialog]'))
  await act(async () => button(dialog, 'Stay').click())
  assert.equal(secret.value, 'synthetic-unsaved-secret')
  await act(async () => button(container, 'Models').click())
  dialog = required(document.querySelector('[role=dialog]'))
  await act(async () => button(dialog, 'Leave').click())
  assert.equal(container.querySelector('input[type=password]'), null)
  assert.equal(button(container, 'Models').getAttribute('aria-current'), 'page')
  await act(async () => button(container, 'Channels').click())
  await act(async () => button(container, 'Create Channel').click())
  assert.equal(
    (container.querySelector('input[type=password]') as HTMLInputElement).value,
    ''
  )
})

test('model fields preserve the CAS draft and require a fresh preview after exact USD price edits', async () => {
  const config: NativeConfiguration = fixture()
  config.bindings.contexts = [
    {
      upstream_id: 1,
      upstream_model: 'model-a',
      protocol: 'openai_chat',
      endpoint: config.routing.upstreams[0].endpoint,
      max_input_tokens: 8192,
      max_output_tokens: 4096,
      contract_revision: 'supplier-doc-2026',
      missing_cached_tokens_is_zero: true,
    },
  ]
  config.routing.routes[0].targets[0].client_protocols = ['openai_chat']
  const previews: Array<Record<string, unknown>> = []
  const writes: Array<Record<string, unknown>> = []
  api.request = (async (options: {
    method: string
    url: string
    data?: Record<string, unknown>
  }) => {
    if (options.method === 'GET') {
      return { data: { success: true, data: config } }
    }
    assert.ok(options.data)
    if (options.url.endsWith('/preview')) {
      assert.equal(options.data.editing, true)
      previews.push(options.data.model as Record<string, unknown>)
      return {
        data: {
          success: true,
          data: { confirmation_digest: String(previews.length).repeat(64) },
        },
      }
    }
    assert.equal(options.url, '/core/v1/admin/routing/models/1')
    writes.push(options.data)
    return {
      data: { success: true, data: { version: { routing: 2, pricing: 2 } } },
    }
  }) as typeof api.request
  const container = await mount('models')
  await until(() => !!container.textContent?.includes('model-a'))
  await act(async () => button(container, 'Edit').click())
  const form = required(container.querySelector('form'))
  assert.equal(
    required(form.querySelector<HTMLDetailsElement>('details:last-of-type'))
      .open,
    false
  )
  const field = (text: string) =>
    required(
      [...form.querySelectorAll('label')]
        .find((item) => item.querySelector('span')?.textContent === text)
        ?.querySelector('input')
    )
  const setter = required(
    Object.getOwnPropertyDescriptor(dom.HTMLInputElement.prototype, 'value')
      ?.set
  )
  const fill = async (label: string, value: string) =>
    act(async () => {
      const input = field(label)
      setter.call(input, value)
      input.dispatchEvent(new Event('input', { bubbles: true }))
    })
  await fill('Model name', 'public-model')
  await fill('Input price (USD / 1M tokens)', '1.234567')
  await act(async () => button(form, 'Preview').click())
  await until(() => !!form.textContent?.includes('Confirm and apply'))
  assert.equal(writes.length, 0)
  await fill('Input price (USD / 1M tokens)', '1.000001')
  assert.equal(form.textContent?.includes('Confirm and apply'), false)
  await act(async () => button(form, 'Preview').click())
  await until(() => previews.length === 2)
  await act(async () => button(form, 'Confirm and apply').click())
  await until(() => writes.length === 1)
  assert.deepEqual(writes[0].expected_version, { routing: 1, pricing: 1 })
  assert.equal(writes[0].confirmation_digest, '2'.repeat(64))
  const prices = writes[0].prices as typeof config.prices.rules
  assert.equal(prices[0].model, 'public-model')
  assert.equal((prices[0].rates as { input: number }).input, 1_000_001)
  assert.equal(prices[0].group_multiplier_ppm, 1_000_000)
  assert.deepEqual(writes[0].contexts, config.bindings.contexts)
  assert.deepEqual(
    (writes[0].route as (typeof config.routing.routes)[0]).targets,
    config.routing.routes[0].targets
  )
})

test('a newly mapped model keeps its supplier contract open while the operator types its revision', async () => {
  const config: NativeConfiguration = fixture()
  config.bindings.contexts = [
    {
      upstream_id: 1,
      upstream_model: 'model-a',
      protocol: 'openai_chat',
      endpoint: config.routing.upstreams[0].endpoint,
      max_input_tokens: 8192,
      max_output_tokens: 4096,
      contract_revision: 'supplier-a',
    },
  ]
  api.request = (async (options: { method: string }) => {
    assert.equal(options.method, 'GET')
    return { data: { success: true, data: config } }
  }) as typeof api.request
  const container = await mount('models')
  await until(() => !!container.textContent?.includes('model-a'))
  await act(async () => button(container, 'Edit').click())
  const form = required(container.querySelector('form'))
  const fields = required(form.querySelector('fieldset'))
  const field = (label: string) =>
    required(
      [...fields.querySelectorAll('label')]
        .find((item) => item.querySelector('span')?.textContent === label)
        ?.querySelector('input')
    )
  const contract = required(fields.querySelector('details'))
  assert.equal(contract.open, false)
  await changeInput(field('Upstream model'), 'new-supplier-model')
  assert.equal(contract.open, true)
  assert.equal(field('Contract revision').value, '')
  assert.equal(field('Maximum input tokens').value, '')
  await changeInput(field('Contract revision'), 's')
  assert.equal(contract.open, true)
  await changeInput(field('Contract revision'), 'supplier-b')
  assert.equal(contract.open, true)
  assert.equal(field('Contract revision').value, 'supplier-b')
})

for (const unknown of [false, true]) {
  test(`batch key import validates distinct lines and clears submitted plaintext: ${
    unknown ? 'unknown result' : 'verified receipt'
  }`, async () => {
    const config = fixture()
    let version = 1
    const writes: Array<Record<string, unknown>> = []
    let keyReads = 0
    api.request = (async (options: {
      method: string
      url: string
      data?: Record<string, unknown>
    }) => {
      if (options.method === 'GET') {
        if (!options.url.endsWith('/keys')) {
          return { data: { success: true, data: config } }
        }
        keyReads++
        return {
          data: {
            success: true,
            data: {
              version: { routing: version, pricing: 1 },
              active_reference: { id: 7, revision: 1 },
              polling: false,
              plaintext_available: false,
              keys: [
                {
                  reference: { id: 7, revision: 1 },
                  label: 'Primary',
                  created_at: '2026-11-16T00:00:00Z',
                  enabled: true,
                },
              ],
            },
          },
        }
      }
      assert.equal(options.url, '/core/v1/admin/routing/channels/1/keys')
      assert.ok(options.data)
      writes.push(options.data)
      version++
      if (unknown) throw new Error('Synthetic response lost')
      return {
        data: {
          success: true,
          data: {
            version: { routing: version, pricing: 1 },
            reference: { id: 8, revision: 1 },
            imported_count: 2,
          },
        },
      }
    }) as typeof api.request
    const container = await mount('channels')
    await until(() =>
      [...container.querySelectorAll('button')].some(
        (item) => item.textContent === 'API Keys'
      )
    )
    await act(async () => button(container, 'API Keys').click())
    await until(
      () =>
        !!document
          .querySelector('[role=dialog]')
          ?.textContent?.includes('Primary')
    )
    const dialog = required(document.querySelector('[role=dialog]'))
    await act(async () => button(dialog, 'Batch import').click())
    const form = required(dialog.querySelector('form'))
    const name = required(form.querySelector('input'))
    const secret = required(form.querySelector('textarea'))
    const inputSetter = required(
      Object.getOwnPropertyDescriptor(dom.HTMLInputElement.prototype, 'value')
        ?.set
    )
    const textSetter = required(
      Object.getOwnPropertyDescriptor(
        dom.HTMLTextAreaElement.prototype,
        'value'
      )?.set
    )
    await act(async () => {
      inputSetter.call(name, 'Pool')
      name.dispatchEvent(new Event('input', { bubbles: true }))
      textSetter.call(secret, 'synthetic-a\nsynthetic-a')
      secret.dispatchEvent(new Event('input', { bubbles: true }))
    })
    await act(async () => button(form, 'Import keys').click())
    assert.equal(writes.length, 0)
    assert.match(dialog.textContent ?? '', /1 to 100 different keys/)
    await act(async () => {
      textSetter.call(secret, 'synthetic-a\n\nsynthetic-b\n')
      secret.dispatchEvent(new Event('input', { bubbles: true }))
    })
    await act(async () => {
      button(form, 'Import keys').click()
      button(form, 'Import keys').click()
    })
    await until(
      () =>
        writes.length === 1 &&
        keyReads >= 2 &&
        !button(form, 'Import keys').disabled
    )
    assert.deepEqual(writes[0], {
      expected_version: { routing: 1, pricing: 1 },
      operation: 'add_many',
      polling: false,
      keys: [
        { label: 'Pool 1', secret: 'synthetic-a' },
        { label: 'Pool 2', secret: 'synthetic-b' },
      ],
    })
    assert.equal(secret.value, '')
    if (unknown) {
      assert.match(dialog.textContent ?? '', /Synthetic response lost/)
      assert.doesNotMatch(dialog.textContent ?? '', /Imported:/)
    } else {
      assert.match(dialog.textContent ?? '', /Imported: 2/)
    }
    await act(async () => {
      textSetter.call(secret, 'synthetic-unsent')
      secret.dispatchEvent(new Event('input', { bubbles: true }))
    })
    await act(async () => button(dialog, 'Single key').click())
    await act(async () => button(dialog, 'Batch import').click())
    assert.equal(required(form.querySelector('textarea')).value, '')
    assert.equal(writes.length, 1)
  })
}

test('channel search explains an empty result and clearing restores the compact channel list', async () => {
  api.request = (async (options: { method: string }) => {
    assert.equal(options.method, 'GET')
    return { data: { success: true, data: fixture() } }
  }) as typeof api.request
  const container = await mount('channels')
  await until(
    () => !!container.querySelector('article[aria-label="Private provider"]')
  )
  const row = required(container.querySelector('article'))
  assert.ok(row.textContent?.includes('OpenAI Chat Completions'))
  assert.equal(required(row.querySelector('details')).open, false)
  assert.equal(button(row, 'Edit').closest('details'), null)
  assert.equal(button(row, 'Test').closest('details'), null)
  const search = required(
    container.querySelector<HTMLInputElement>('input[aria-label="Search"]')
  )
  await changeInput(search, 'missing-provider')
  assert.equal(container.querySelector('article'), null)
  assert.ok(container.textContent?.includes('No results found'))
  await act(async () => button(container, 'Clear').click())
  assert.equal(search.value, '')
  assert.ok(container.querySelector('article[aria-label="Private provider"]'))
})

for (const section of ['models', 'vendors'] as const) {
  test(`empty ${section} explain the next step and keep creation available`, async () => {
    const config: NativeConfiguration = fixture()
    if (section === 'models') config.routing.routes = []
    api.request = (async (options: { method: string }) => {
      assert.equal(options.method, 'GET')
      return { data: { success: true, data: config } }
    }) as typeof api.request
    const container = await mount(section)
    await until(
      () =>
        !!container
          .querySelector('[role="status"]')
          ?.textContent?.includes('No Data')
    )
    const empty = required(container.querySelector('[role="status"]'))
    assert.match(empty.textContent ?? '', /No Data/)
    assert.match(
      empty.textContent ?? '',
      section === 'models'
        ? /Add a new model to the system/
        : /Add a new vendor to the system/
    )
    assert.equal(empty.querySelector('button'), null)
    const create = button(
      container,
      section === 'models' ? 'Add Model' : 'Create'
    )
    assert.equal(create.disabled, false)
    await act(async () => create.click())
    assert.ok(
      container.querySelector(section === 'models' ? 'form' : 'textarea')
    )
  })

  test(`unmatched ${section} search can be cleared to recover the list`, async () => {
    const config: NativeConfiguration = fixture()
    config.bindings.vendors = {
      1: { id: 1, name: 'Synthetic vendor', description: '' },
    }
    const name = section === 'models' ? 'model-a' : 'Synthetic vendor'
    api.request = (async (options: { method: string }) => {
      assert.equal(options.method, 'GET')
      return { data: { success: true, data: config } }
    }) as typeof api.request
    const container = await mount(section)
    await until(() => !!container.textContent?.includes(name))
    const search = required(
      container.querySelector<HTMLInputElement>('input[aria-label="Search"]')
    )
    await changeInput(search, 'missing-result')
    const empty = required(container.querySelector('[role="status"]'))
    assert.match(empty.textContent ?? '', /No results found/)
    assert.match(empty.textContent ?? '', /Try a different search or clear/)
    assert.equal(container.textContent?.includes(name), false)
    await act(async () => button(empty, 'Clear filters').click())
    assert.equal(search.value, '')
    assert.equal(container.querySelector('[role="status"]'), null)
    assert.ok(container.textContent?.includes(name))
  })
}

test('channel edits preserve stale drafts and require explicit reload after a configuration version changes', async () => {
  let config: NativeConfiguration = fixture()
  let writes = 0
  api.request = (async (options: { method: string }) => {
    if (options.method !== 'GET') writes++
    return { data: { success: true, data: config } }
  }) as typeof api.request
  const container = await mount('channels')
  await until(() => !!container.textContent?.includes('Private provider'))
  await act(async () => button(container, 'Edit').click())
  const form = required(container.querySelector('form'))
  const name = required(form.querySelector('input'))
  const setter = required(
    Object.getOwnPropertyDescriptor(dom.HTMLInputElement.prototype, 'value')
      ?.set
  )
  await act(async () => {
    setter.call(name, 'Unpublished channel')
    name.dispatchEvent(new Event('input', { bubbles: true }))
  })
  config = {
    ...config,
    version: { routing: 2, pricing: 1 },
    routing: { ...config.routing, version: 2 },
    bindings: { ...config.bindings, names: { 1: 'Updated provider' } },
  }
  await act(async () => button(container, 'Refresh').click())
  await until(() => !!form.textContent?.includes('Configuration changed.'))
  assert.equal(name.value, 'Unpublished channel')
  assert.deepEqual(
    parseNativeDocument(required(form.querySelector('textarea')).value)
      .expected_version,
    { routing: 1, pricing: 1 }
  )
  assert.equal(button(form, 'Save').disabled, true)
  await act(async () => {
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
  })
  assert.equal(writes, 0)

  await act(async () => button(form, 'Reload editor').click())
  let dialog = required(document.querySelector('[role=dialog]'))
  await act(async () => button(dialog, 'Stay').click())
  assert.equal(name.value, 'Unpublished channel')
  await act(async () => button(form, 'Reload editor').click())
  dialog = required(document.querySelector('[role=dialog]'))
  await act(async () => button(dialog, 'Leave').click())
  assert.equal(name.value, 'Updated provider')
  assert.equal(button(form, 'Save').disabled, false)
  assert.deepEqual(
    parseNativeDocument(required(form.querySelector('textarea')).value)
      .expected_version,
    { routing: 2, pricing: 1 }
  )
  assert.equal(writes, 0)
})

test('model confirmation stops when configuration changes and reload requires a new preview', async () => {
  let config: NativeConfiguration = fixture()
  const previews: Record<string, unknown>[] = []
  let writes = 0
  api.request = (async (options: {
    method: string
    url: string
    data?: Record<string, unknown>
  }) => {
    if (options.method === 'GET') {
      return { data: { success: true, data: config } }
    }
    if (options.url.endsWith('/models/preview')) {
      previews.push(required(options.data))
      return {
        data: { success: true, data: { confirmation_digest: 'a'.repeat(64) } },
      }
    }
    writes++
    return { data: { success: true, data: {} } }
  }) as typeof api.request
  const container = await mount('models')
  await until(() => !!container.textContent?.includes('model-a'))
  await act(async () => button(container, 'Edit').click())
  const form = required(container.querySelector('form'))
  await act(async () => {
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
  })
  await until(() => previews.length === 1)
  assert.equal(button(form, 'Confirm and apply').disabled, false)
  config = {
    ...config,
    version: { routing: 2, pricing: 2 },
    routing: { ...config.routing, version: 2 },
    prices: { ...config.prices, version: 2 },
  }
  await act(async () => button(container, 'Refresh').click())
  await until(() => !!form.textContent?.includes('Configuration changed.'))
  assert.equal(button(form, 'Confirm and apply').disabled, true)
  assert.equal(form.querySelector('pre[aria-label=Preview]'), null)
  await act(async () => {
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
  })
  assert.equal(writes, 0)
  assert.equal(previews.length, 1)
  await act(async () => button(form, 'Reload editor').click())
  assert.equal(button(form, 'Preview').disabled, false)
  await act(async () => {
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
  })
  await until(() => previews.length === 2)
  assert.deepEqual(
    (previews[1].model as Record<string, unknown>).expected_version,
    { routing: 2, pricing: 2 }
  )
  assert.equal(writes, 0)
})

test('model timeout edits keep the saved value, block a short JSON draft and show a newer server minimum', async () => {
  const config: NativeConfiguration = {
    ...fixture(),
    model_limits: { total_timeout_ms: 120_000 },
  }
  let posts = 0
  api.request = (async (options: { method: string }) => {
    if (options.method === 'GET') {
      return { data: { success: true, data: config } }
    }
    posts++
    throw {
      response: {
        data: {
          success: false,
          code: 'CHANNEL_TARGET_TIMEOUT_TOO_SHORT',
          message: 'Target timeout is below the global deadline',
          data: { required_min_timeout_ms: 180_000 },
        },
      },
    }
  }) as typeof api.request
  const container = await mount('models')
  await until(() => !!container.textContent?.includes('model-a'))
  await act(async () => button(container, 'Edit').click())
  const form = required(container.querySelector('form'))
  const input = required(
    [...form.querySelectorAll('label')]
      .find(
        (item) => item.querySelector('span')?.textContent === 'Timeout (ms)'
      )
      ?.querySelector('input')
  )
  assert.equal(input.value, '30000')
  assert.equal(input.min, '120000')
  assert.equal(input.getAttribute('aria-invalid'), 'true')
  assert.match(form.textContent ?? '', /larger values do not extend execution/)
  // A programmatic submit also covers the advanced JSON path beyond HTML min validation.
  await act(async () =>
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
  )
  assert.equal(posts, 0)
  assert.match(
    container.textContent ?? '',
    /Target timeout must be at least 120000 ms/
  )
  await changeInput(input, '120000')
  assert.equal(input.getAttribute('aria-invalid'), null)
  await act(async () =>
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
  )
  await until(
    () =>
      posts === 1 &&
      !!container.textContent?.includes(
        'Target timeout must be at least 180000 ms'
      )
  )
  assert.equal(button(form, 'Preview').disabled, false)
  assert.equal(form.textContent?.includes('Confirm and apply'), false)
  assert.equal(config.routing.routes[0].targets[0].timeout_ms, 30000)
})

test('channel capability expiry and failed refresh preserve the draft and CAS until recovery, while explicit withdrawal returns to read only', async () => {
  const config: NativeConfiguration = fixture()
  config.bindings.contexts = [
    {
      upstream_id: 1,
      upstream_model: 'model-a',
      protocol: 'openai_chat',
      endpoint: config.routing.upstreams[0].endpoint,
      max_input_tokens: 8192,
      max_output_tokens: 4096,
      contract_revision: 'synthetic-capability-draft',
    },
  ]
  const writes: Record<string, unknown>[] = []
  let reads = 0
  api.request = (async (options: {
    method: string
    data?: Record<string, unknown>
  }) => {
    if (options.method === 'GET') {
      reads++
      return { data: { success: true, data: config } }
    }
    writes.push(required(options.data))
    return {
      data: { success: true, data: { version: { routing: 2, pricing: 1 } } },
    }
  }) as typeof api.request
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const container = await mount(
    'channels',
    true,
    accountingCapabilities,
    client,
    true
  )
  await until(() => !!container.textContent?.includes('Private provider'))
  await act(async () => button(container, 'Edit').click())
  const form = required(
    container.querySelector('form[aria-label="Edit Channel"]')
  )
  const name = required(form.querySelector('input'))
  await changeInput(name, 'Draft retained while capabilities expire')
  const draft = required(form.querySelector('textarea')).value
  const evidence = required(client.getQueryData(['channel-ops']))

  await act(async () => {
    client.setQueryData(['channel-ops'], evidence, {
      updatedAt: Date.now() - 60_001,
    })
  })
  await until(() => button(form, 'Save').disabled)
  assert.equal(container.querySelector('form[aria-label="Edit Channel"]'), form)
  assert.equal(required(form.querySelector('textarea')).value, draft)
  assert.equal(name.value, 'Draft retained while capabilities expire')
  assert.equal(button(container, 'Refresh').disabled, false)
  assert.equal(button(container, 'Refresh capabilities').disabled, false)
  assert.equal(button(form, 'Cancel').disabled, false)
  for (const label of ['Disable', 'Test', 'Sync', 'Delete', 'API Keys']) {
    assert.equal(button(container, label).disabled, true)
  }
  await act(async () =>
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
  )
  assert.equal(writes.length, 0)
  const readsBeforeRefresh = reads
  await act(async () => button(container, 'Refresh').click())
  await until(() => reads > readsBeforeRefresh)
  assert.equal(required(form.querySelector('textarea')).value, draft)

  const successfulAdapter = api.defaults.adapter
  api.defaults.adapter = async () => {
    throw new Error('Synthetic capability refresh failure')
  }
  await act(async () => button(container, 'Refresh capabilities').click())
  await until(() => client.getQueryState(['channel-ops'])?.status === 'error')
  assert.equal(container.querySelector('form[aria-label="Edit Channel"]'), form)
  assert.equal(required(form.querySelector('textarea')).value, draft)
  assert.equal(button(form, 'Save').disabled, true)
  await act(async () =>
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
  )
  assert.equal(writes.length, 0)

  api.defaults.adapter = successfulAdapter
  await act(async () => button(container, 'Refresh capabilities').click())
  await until(() => !button(form, 'Save').disabled)
  assert.equal(container.querySelector('form[aria-label="Edit Channel"]'), form)
  assert.equal(required(form.querySelector('textarea')).value, draft)
  await act(async () => button(form, 'Save').click())
  await until(
    () =>
      writes.length === 1 &&
      !container.querySelector('form[aria-label="Edit Channel"]')
  )
  assert.equal(writes[0].name, 'Draft retained while capabilities expire')
  assert.deepEqual(writes[0].expected_version, { routing: 1, pricing: 1 })
  assert.equal(Object.hasOwn(writes[0], 'secret'), false)

  await act(async () => {
    client.setQueryData(['channel-ops'], {
      success: true,
      data: {
        native_configuration: false,
        native_control: false,
        native_secrets: false,
      },
    })
  })
  await until(
    () => !!container.textContent?.includes('Channels are read-only.')
  )
  await until(() => !!container.textContent?.includes('Total: 0'))
  assert.equal(
    container.querySelector('section[aria-label="Configuration"]'),
    null
  )
  await until(
    () => client.isFetching({ queryKey: ['channels', 'read-only'] }) === 0
  )
  assert.equal(button(container, 'Configuration').disabled, true)
  assert.equal(writes.length, 1)
})

test('an open key manager preserves unsubmitted credentials and blocks writes during capability expiry', async () => {
  const config = fixture()
  const writes: Record<string, unknown>[] = []
  api.request = (async (options: {
    method: string
    url: string
    data?: Record<string, unknown>
  }) => {
    if (options.method === 'GET') {
      return {
        data: {
          success: true,
          data: options.url.endsWith('/keys')
            ? {
                version: config.version,
                active_reference: { id: 7, revision: 1 },
                polling: true,
                plaintext_available: false,
                keys: [
                  {
                    reference: { id: 7, revision: 1 },
                    label: 'Primary',
                    created_at: '2026-11-18T00:00:00Z',
                    enabled: true,
                  },
                ],
              }
            : config,
        },
      }
    }
    writes.push(required(options.data))
    return {
      data: {
        success: true,
        data: {
          version: { routing: 2, pricing: 1 },
          reference: { id: 8, revision: 1 },
        },
      },
    }
  }) as typeof api.request
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const container = await mount(
    'channels',
    true,
    accountingCapabilities,
    client,
    true
  )
  await until(() => !!container.textContent?.includes('Private provider'))
  await act(async () => button(container, 'API Keys').click())
  await until(() => !!document.querySelector('[role=dialog] form'))
  const dialog = required(document.querySelector('[role=dialog]'))
  const form = required(dialog.querySelector('form'))
  const name = required(
    form.querySelector<HTMLInputElement>('input:not([type=password])')
  )
  const secret = required(
    form.querySelector<HTMLInputElement>('input[type=password]')
  )
  await changeInput(name, 'Unsubmitted key')
  await changeInput(secret, 'synthetic-not-a-real-key')
  const evidence = required(client.getQueryData(['channel-ops']))
  await act(async () =>
    client.setQueryData(['channel-ops'], evidence, {
      updatedAt: Date.now() - 60_001,
    })
  )
  await until(() => button(form, 'Add').disabled)
  assert.equal(document.querySelector('[role=dialog]'), dialog)
  assert.equal(secret.value, 'synthetic-not-a-real-key')
  assert.equal(name.value, 'Unsubmitted key')
  assert.equal(secret.type, 'password')
  assert.equal(dialog.textContent?.includes('synthetic-not-a-real-key'), false)
  assert.equal(dialog.textContent?.includes('Saving...'), false)
  await act(async () =>
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
  )
  assert.equal(writes.length, 0)
  assert.equal(secret.value, 'synthetic-not-a-real-key')
  await act(async () => client.setQueryData(['channel-ops'], evidence))
  await until(() => !button(form, 'Add').disabled)
  await act(async () => button(form, 'Add').click())
  await until(() => writes.length === 1 && !button(form, 'Add').disabled)
  assert.deepEqual(writes[0], {
    expected_version: { routing: 1, pricing: 1 },
    operation: 'add',
    label: 'Unsubmitted key',
    secret: 'synthetic-not-a-real-key',
    activate: false,
  })
  assert.equal(secret.value, '')
})

test('batch routing retains local values through capability expiry without claiming to save', async () => {
  const config = fixture()
  const writes: Record<string, unknown>[] = []
  api.request = (async (options: {
    method: string
    data?: Record<string, unknown>
  }) => {
    if (options.method === 'GET') {
      return { data: { success: true, data: config } }
    }
    writes.push(required(options.data))
    return {
      data: {
        success: true,
        data: {
          version: { routing: 2, pricing: 1 },
          updated: 1,
          target_count: 1,
        },
      },
    }
  }) as typeof api.request
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const container = await mount(
    'channels',
    true,
    accountingCapabilities,
    client,
    true
  )
  await until(() => !!container.textContent?.includes('Private provider'))
  await act(async () =>
    (
      required(container.querySelector('[role=checkbox]')) as HTMLElement
    ).click()
  )
  const actions = required(
    container.querySelector('[data-slot=channel-batch-actions]')
  )
  await act(async () => button(actions, 'Edit routing').click())
  const form = required(
    container.querySelector('form[aria-label="Edit routing"]')
  )
  const priority = required(
    form.querySelector<HTMLInputElement>('input[name=priority]')
  )
  await changeInput(priority, '7')
  assert.equal(button(form, 'Apply').disabled, false)
  const evidence = required(client.getQueryData(['channel-ops']))
  await act(async () =>
    client.setQueryData(['channel-ops'], evidence, {
      updatedAt: Date.now() - 60_001,
    })
  )
  await until(() => button(form, 'Apply').disabled)
  assert.equal(container.querySelector('form[aria-label="Edit routing"]'), form)
  assert.equal(priority.value, '7')
  assert.equal(form.textContent?.includes('Saving...'), false)
  assert.equal(button(form, 'Cancel').disabled, false)
  await act(async () =>
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
  )
  assert.equal(writes.length, 0)
  await act(async () => client.setQueryData(['channel-ops'], evidence))
  await until(() => !button(form, 'Apply').disabled)
  await act(async () => button(form, 'Apply').click())
  await until(() => writes.length === 1)
  assert.deepEqual(writes[0], {
    expected_version: { routing: 1, pricing: 1 },
    ids: [1],
    patch: { priority: 7 },
  })
})

test('open balance views preserve observations and configuration drafts while capability expiry blocks probe and save', async () => {
  const config: NativeConfiguration = {
    ...fixture(),
    bindings: {
      ...fixture().bindings,
      balance_probes: {
        1: {
          path: '/balance',
          json_pointer: '/available',
          unit_label: 'points',
        },
      },
    },
  }
  const writes: Record<string, unknown>[] = []
  let probes = 0
  api.request = (async (options: {
    method: string
    url: string
    data?: Record<string, unknown>
  }) => {
    if (options.url.endsWith('/balance')) probes++
    if (options.method === 'GET') {
      return {
        data: {
          success: true,
          data: options.url.endsWith('/counters')
            ? { ...countersFixture, balance: balanceFixture }
            : config,
        },
      }
    }
    writes.push(required(options.data))
    return {
      data: { success: true, data: { version: { routing: 2, pricing: 1 } } },
    }
  }) as typeof api.request
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const container = await mount(
    'channels',
    true,
    accountingCapabilities,
    client,
    true
  )
  await until(() => !!container.textContent?.includes('Private provider'))
  await act(async () =>
    button(container, i18n.t('channelBalance.title')).click()
  )
  await until(() => !!document.querySelector('[role=dialog] dl'))
  let dialog = required(document.querySelector('[role=dialog]'))
  assert.equal(button(dialog, 'Update Balance').disabled, false)
  const evidence = required(client.getQueryData(['channel-ops']))
  await act(async () =>
    client.setQueryData(['channel-ops'], evidence, {
      updatedAt: Date.now() - 60_001,
    })
  )
  await until(() => button(dialog, 'Update Balance').disabled)
  assert.equal(document.querySelector('[role=dialog]'), dialog)
  assert.ok(dialog.textContent?.includes(balanceFixture.balance))
  await act(async () => button(dialog, 'Update Balance').click())
  assert.equal(probes, 0)
  assert.equal(button(dialog, 'Close').disabled, false)
  await act(async () => button(dialog, 'Close').click())
  await act(async () => client.setQueryData(['channel-ops'], evidence))
  await until(() =>
    [...container.querySelectorAll('button')].some(
      (item) => item.textContent === i18n.t('channelBalance.configure')
    )
  )
  await act(async () =>
    button(container, i18n.t('channelBalance.configure')).click()
  )
  dialog = required(document.querySelector('[role=dialog]'))
  const form = required(dialog.querySelector('form'))
  const path = required(form.querySelector('input'))
  await changeInput(path, '/account/available')
  await act(async () =>
    client.setQueryData(['channel-ops'], evidence, {
      updatedAt: Date.now() - 60_001,
    })
  )
  await until(() => button(dialog, 'Save').disabled)
  assert.equal(document.querySelector('[role=dialog]'), dialog)
  assert.equal(path.value, '/account/available')
  assert.equal(button(dialog, 'Disable').disabled, true)
  assert.equal(button(dialog, 'Cancel').disabled, false)
  await act(async () =>
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
  )
  assert.equal(writes.length, 0)
  await act(async () => client.setQueryData(['channel-ops'], evidence))
  await until(() => !button(dialog, 'Save').disabled)
  await act(async () => button(dialog, 'Save').click())
  await until(
    () => writes.length === 1 && !document.querySelector('[role=dialog]')
  )
  assert.deepEqual(writes[0], {
    expected_version: { routing: 1, pricing: 1 },
    probe: {
      path: '/account/available',
      json_pointer: '/available',
      unit_label: 'points',
    },
  })
})

test('read-only configuration leaves browsing available and disables new model, vendor and price edits', async () => {
  const configuration: NativeConfiguration = fixture()
  configuration.bindings.vendors = {
    '1': { id: 1, name: 'Synthetic vendor', description: 'Fixture' },
  }
  configuration.bindings.price_drafts = {
    '1': {
      id: 1,
      name: 'Synthetic draft',
      base_pricing_version: 1,
      prices: configuration.prices,
      updated_by: 501,
      updated_at: 1,
    },
  }
  api.request = (async () => ({
    data: { success: true, data: configuration },
  })) as typeof api.request
  for (const section of ['models', 'vendors', 'prices'] as const) {
    const container = await mount(
      section,
      true,
      accountingCapabilities,
      new QueryClient({ defaultOptions: { queries: { retry: false } } }),
      false,
      false
    )
    await until(() => !!container.querySelector('input[aria-label="Search"]'))
    for (const element of container.querySelectorAll<HTMLButtonElement>(
      'button'
    )) {
      if (
        [
          'Add Model',
          'Create',
          'Edit',
          'Delete',
          'Save draft',
          'Preview',
        ].includes(element.textContent ?? '')
      ) {
        assert.equal(
          element.disabled,
          true,
          `${section}: ${element.textContent}`
        )
      }
    }
    assert.equal(button(container, 'Refresh').disabled, false)
    assert.equal(button(container, 'Channels').disabled, false)
    if (section === 'prices') {
      assert.ok(container.querySelector('[data-slot="native-price-rule-list"]'))
      const raw = [...container.querySelectorAll('details')].find(
        (item) => item.querySelector('summary')?.textContent === 'Raw JSON'
      )
      assert.ok(raw)
      assert.equal(raw.open, false)
      const rawDocument = raw.querySelector('pre')?.textContent
      assert.ok(rawDocument)
      assert.equal(parseNativeDocument(rawDocument).version, 1)
    }
    await dispose?.()
    dispose = undefined
    document.body.replaceChildren()
  }
})
