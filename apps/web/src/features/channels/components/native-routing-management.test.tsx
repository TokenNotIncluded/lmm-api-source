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
const en = await import('@/i18n/locales/en.json')
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: en.default } })
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
  accounting: Partial<typeof accountingCapabilities> & {
    native_channel_observations?: boolean
  } = accountingCapabilities,
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
  return document.body
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

test('channel group prices copy only a matching default speed, preview, publish and reload', async () => {
  const config = fixture()
  config.routing.routes[0].group = 'vip'
  config.routing.groups.push({ name: 'vip', enabled: true })
  const standard = config.prices.rules[0]
  config.prices.rules.push({
    ...standard,
    id: 2,
    group: 'vip',
    rates: { ...standard.rates, input: 42 },
  })
  config.prices.rules.push({ ...standard, id: 3, speed: 'fast' })
  const original = structuredClone(config.prices.rules)
  let writes = 0
  const digest = 'e'.repeat(64)
  api.request = (async (options: {
    url: string
    method: string
    data?: Record<string, unknown>
  }) => {
    const ok = (data: unknown) => ({ data: { success: true, data } })
    if (options.method === 'GET') return ok(config)
    const body = required(options.data)
    const next = body.prices as typeof config.prices
    assert.deepEqual(body.expected_version, { routing: 1, pricing: 1 })
    assert.deepEqual(next.rules[0], original[0])
    assert.deepEqual(next.rules[2], original[2])
    assert.equal(next.rules[1].id, 2)
    assert.equal(next.rules[1].group, 'vip')
    assert.equal(next.rules[1].rates.input, 2)
    assert.equal(next.rules[1].group_multiplier_ppm, 1_500_000)
    if (options.method === 'POST') {
      assert.equal(options.url, '/core/v1/admin/routing/prices/preview')
      return ok({
        before: config.prices,
        after: next,
        confirmation_digest: digest,
      })
    }
    assert.equal(options.method, 'PUT')
    assert.equal(options.url, '/core/v1/admin/routing/prices')
    assert.equal(body.confirmation_digest, digest)
    config.prices = next
    config.version = { routing: 2, pricing: 2 }
    config.routing.version = 2
    writes++
    return ok({ version: config.version })
  }) as typeof api.request
  const container = await mount('channels')
  await until(() =>
    [...container.querySelectorAll('button')].some(
      (item) => item.textContent === 'Edit'
    )
  )
  const openGroup = async () => {
    await act(async () => button(container, 'Edit').click())
    const form = required(
      container.querySelector('form[aria-label="Edit Channel"]')
    )
    await act(async () => button(form, 'Edit group prices').click())
    return required(
      container.querySelector('form[aria-label="Edit group prices"]')
    )
  }
  let form = await openGroup()
  const multiplier = () =>
    required(
      [...form.querySelectorAll('label')]
        .find((label) => label.textContent === 'Group multiplier')
        ?.querySelector('input')
    )
  await act(async () =>
    button(form, 'Copy prices from the default group').click()
  )
  await changeInput(multiplier(), '1.5')
  await act(async () => button(form, 'Preview').click())
  assert.ok(
    form.textContent?.includes('Confirm and apply'),
    container.textContent ?? ''
  )
  await until(() => !!form.textContent?.includes('Confirm and apply'))
  assert.equal(writes, 0)
  assert.match(
    required(form.querySelector('section[aria-label="Preview"]')).textContent ??
      '',
    /0\.000003/
  )
  await act(async () => button(form, 'Confirm and apply').click())
  await until(
    () =>
      writes === 1 &&
      !container.querySelector('form[aria-label="Edit group prices"]')
  )
  form = await openGroup()
  assert.equal(multiplier().value, '1.5')
})

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
  const assertChannelStatus = (expected: 'Enabled' | 'Disabled') => {
    const row = required(
      container.querySelector('article[aria-label="Private provider"]')
    )
    assert.ok(
      [...row.querySelectorAll('p')].some((p) => p.textContent === '#1')
    )
    assert.ok(
      [...row.querySelectorAll('span')].some(
        (span) => span.textContent === expected
      )
    )
  }
  assertChannelStatus('Enabled')
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
  assertChannelStatus('Disabled')
  await act(async () => button(container, 'Enable').click())
  await until(
    () =>
      writes.length === 2 &&
      [...container.querySelectorAll('button')].some(
        (element) => element.textContent === 'Disable'
      )
  )
  assertChannelStatus('Enabled')
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
  const diff = required(
    form.querySelector('section[aria-label="Preview changes"]')
  )
  assert.match(diff.textContent ?? '', /model-a/)
  assert.match(diff.textContent ?? '', /30000 → 120000/)
  assert.equal(writes.length, 0)
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

test('a new channel selection starts without the previous batch tag draft', async () => {
  const config = fixture()
  config.routing.upstreams.push({ ...config.routing.upstreams[0], id: 2 })
  config.bindings.names[2] = 'Secondary provider'
  api.request = (async (options: { method: string }) => {
    assert.equal(options.method, 'GET')
    return { data: { success: true, data: config } }
  }) as typeof api.request

  const container = await mount('channels')
  const checkbox = (name: string) =>
    required(
      container.querySelector<HTMLElement>(
        `[role=checkbox][aria-label="${name}"]`
      )
    )
  const tagForm = () =>
    container.querySelector('form[aria-label="Set tag for selected channels"]')
  await until(() => container.querySelectorAll('[role=checkbox]').length === 3)
  await act(async () => checkbox('Private provider').click())
  await act(async () => button(container, 'Tag').click())
  await changeInput(required(tagForm()?.querySelector('input')), 'Primary only')

  await act(async () => checkbox('Private provider').click())
  await act(async () => checkbox('Secondary provider').click())
  assert.ok(!tagForm(), 'The old tag editor reopened for another channel')
  await act(async () => button(container, 'Tag').click())
  assert.equal(required(tagForm()?.querySelector('input')).value, '')
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
  await until(() => !!document.querySelector('[role=dialog] dl'))
  const dialog = required(document.querySelector('[data-slot=dialog-content]'))
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
    const dialog = required(
      document.querySelector('[data-slot=dialog-content]')
    )
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
    button(
      required(document.querySelector('[data-slot=dialog-content]')),
      'Close'
    ).click()
  )
  assert.equal(signals[0]?.aborted, true)
  await act(async () => finish?.())
  assert.equal(document.querySelector('[role=dialog]'), null)
  assert.equal(container.textContent?.includes('9,007,199,254,740,993'), false)
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
  const dialog = required(document.querySelector('[data-slot=dialog-content]'))
  await until(() => dialog.querySelectorAll('dd').length === 6)
  for (const value of dialog.querySelectorAll('dd')) {
    assert.match(value.textContent ?? '', /^0(?: Credits)?$/)
  }
  assert.equal(dialog.querySelector('[role=alert]'), null)
})

test('Sub2API automatic balances refresh individually and in a batch without clearing failed observations', async () => {
  const { AxiosError, AxiosHeaders } = await import('axios')
  const config = {
    ...fixture(),
    bindings: {
      ...fixture().bindings,
      legacy_balance: { 1: { channel_type: 59 }, 2: { channel_type: 59 } },
    },
  }
  config.routing.upstreams.push({ ...config.routing.upstreams[0], id: 2 })
  config.bindings.names[2] = 'Quota provider'
  const observation = {
    balance: '42.5',
    unit_label: 'USD',
    balance_updated_time: 1_700_000_000,
    recorded_at: 1_700_000_000,
    kind: 'provider',
    source_channel_id: null,
  }
  const calls: number[] = []
  let failAll = false
  let finishSingle: (() => void) | undefined
  api.request = (async (options: { url: string; method: string }) => {
    if (options.url.endsWith('/counters')) {
      return {
        data: {
          success: true,
          data: { ...countersFixture, balance: observation },
        },
      }
    }
    if (!options.url.endsWith('/balance')) {
      return { data: { success: true, data: config } }
    }
    assert.equal(options.method, 'GET')
    const id = Number(options.url.split('/').at(-2))
    calls.push(id)
    if (calls.length === 1) {
      await new Promise<void>((resolve) => {
        finishSingle = resolve
      })
    }
    if (id === 2 || failAll) {
      throw new AxiosError(
        'private provider detail',
        'ERR_BAD_RESPONSE',
        undefined,
        undefined,
        {
          status: 502,
          statusText: 'unavailable',
          headers: {},
          config: { headers: new AxiosHeaders() },
          data: {},
        }
      )
    }
    return { data: { success: true, data: observation } }
  }) as typeof api.request
  const container = await mount('channels')
  await until(() => !!container.textContent?.includes('Quota provider'))
  await act(async () => button(container, 'Provider balance').click())
  await until(() => !!document.querySelector('[role=dialog] dl'))
  let dialog = required(document.querySelector('[data-slot=dialog-content]'))
  assert.equal(button(dialog, 'Update Balance').disabled, false)
  const singleUpdate = button(dialog, 'Update Balance')
  await act(async () => {
    singleUpdate.click()
    singleUpdate.click()
  })
  await until(() => !!finishSingle)
  assert.deepEqual(calls, [1])
  assert.equal(singleUpdate.disabled, true)
  await act(async () => required(finishSingle)())
  await until(() => !singleUpdate.disabled)
  assert.deepEqual(calls, [1])
  assert.equal(dialog.querySelector('.text-2xl')?.textContent, '42.5 USD')
  await act(async () => button(dialog, 'Close').click())
  await act(async () => {
    ;(container.querySelector('[role=checkbox]') as HTMLButtonElement).click()
  })
  const toolbar = required(
    container.querySelector('[data-slot=channel-batch-actions]')
  )
  await act(async () => button(toolbar, 'Refresh selected balances').click())
  dialog = required(document.querySelector('[data-slot=dialog-content]'))
  await act(async () => button(dialog, 'Update Balance').click())
  await until(
    () =>
      calls.length === 3 &&
      !!dialog.textContent?.includes('Current balance unknown')
  )
  assert.deepEqual(calls, [1, 1, 2])
  assert.ok(dialog.textContent?.includes('42.5 USD'))
  assert.equal(dialog.textContent?.includes('private provider detail'), false)
  assert.equal(dialog.textContent?.includes('0 USD'), false)
  failAll = true
  await act(async () => button(dialog, 'Update Balance').click())
  await until(
    () =>
      calls.length === 5 &&
      dialog.querySelectorAll('[role=status]').length === 2
  )
  assert.ok(dialog.textContent?.includes('42.5 USD'))
  assert.ok(
    dialog.textContent?.includes('last recorded balance has not changed')
  )
  await act(async () => button(dialog, 'Close').click())
})

test('Sub2API batch late responses do not enter a filtered replacement selection', async () => {
  const config = {
    ...fixture(),
    bindings: {
      ...fixture().bindings,
      legacy_balance: {
        1: { channel_type: 59 },
        2: { channel_type: 59 },
        3: { channel_type: 59 },
      },
    },
  }
  config.routing.upstreams.push(
    { ...config.routing.upstreams[0], id: 2 },
    { ...config.routing.upstreams[0], id: 3 }
  )
  config.bindings.names[2] = 'Quota provider'
  config.bindings.names[3] = 'Filtered provider'
  const calls: number[] = []
  let finishOld: (() => void) | undefined
  let oldSignal: AbortSignal | undefined
  api.request = (async (options: {
    url: string
    method: string
    signal?: AbortSignal
  }) => {
    if (!options.url.endsWith('/balance')) {
      return { data: { success: true, data: config } }
    }
    assert.equal(options.method, 'GET')
    const id = Number(options.url.split('/').at(-2))
    calls.push(id)
    if (id === 1) {
      oldSignal = options.signal
      // Deliberately return even after abort, like an already received response.
      await new Promise<void>((resolve) => {
        finishOld = resolve
      })
    }
    return {
      data: {
        success: true,
        data: {
          balance: id === 3 ? '0' : '999.5',
          unit_label: 'USD',
          balance_updated_time: 1_700_000_000,
          recorded_at: 1_700_000_000,
          kind: 'provider',
          source_channel_id: null,
        },
      },
    }
  }) as typeof api.request
  const container = await mount('channels')
  await until(() => !!container.textContent?.includes('Filtered provider'))
  await act(async () =>
    required(
      container.querySelector<HTMLButtonElement>('[role=checkbox]')
    ).click()
  )
  let toolbar = required(
    container.querySelector('[data-slot=channel-batch-actions]')
  )
  await act(async () => button(toolbar, 'Refresh selected balances').click())
  let dialog = required(document.querySelector('[data-slot=dialog-content]'))
  assert.equal(dialog.querySelectorAll('li').length, 3)
  await act(async () => button(dialog, 'Update Balance').click())
  await until(() => !!finishOld)
  assert.deepEqual(calls, [1])
  await act(async () => button(dialog, 'Close').click())
  assert.equal(oldSignal?.aborted, true)
  await changeInput(
    required(
      container.querySelector<HTMLInputElement>('input[aria-label="Search"]')
    ),
    'Filtered provider'
  )
  await until(() => !container.textContent?.includes('Quota provider'))
  assert.equal(
    container.querySelector('[data-slot=channel-batch-actions]'),
    null
  )
  await act(async () =>
    required(
      container.querySelector<HTMLButtonElement>('[role=checkbox]')
    ).click()
  )
  toolbar = required(
    container.querySelector('[data-slot=channel-batch-actions]')
  )
  await act(async () => button(toolbar, 'Refresh selected balances').click())
  dialog = required(document.querySelector('[data-slot=dialog-content]'))
  await act(async () => button(dialog, 'Update Balance').click())
  await until(() => !!dialog.textContent?.includes('0 USD'))
  await act(async () => required(finishOld)())
  assert.deepEqual(calls, [1, 3])
  assert.equal(dialog.querySelectorAll('li').length, 1)
  assert.ok(dialog.textContent?.includes('Filtered provider'))
  assert.ok(dialog.textContent?.includes('0 USD'))
  assert.equal(dialog.textContent?.includes('999.5 USD'), false)
  assert.equal(dialog.textContent?.includes('Private provider'), false)
  assert.equal(dialog.textContent?.includes('Quota provider'), false)
  await act(async () => button(dialog, 'Close').click())
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
    const dialog = required(
      document.querySelector('[data-slot=dialog-content]')
    )
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
    const dialog = required(
      document.querySelector('[data-slot=dialog-content]')
    )
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
  let dialog = required(document.querySelector('[data-slot=dialog-content]'))
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
  dialog = required(document.querySelector('[data-slot=dialog-content]'))
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
    button(
      required(document.querySelector('[data-slot=dialog-content]')),
      'Close'
    ).click()
  )
  assert.equal(readSignal?.aborted, true)
  await act(async () => finishRead?.())
  assert.equal(document.querySelector('[role=dialog]'), null)
  await act(async () => button(container, 'Configure balance query').click())
  const dialog = required(document.querySelector('[data-slot=dialog-content]'))
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
    await until(() => !!document.querySelector('[role=dialog] dl'))
    const dialog = required(
      document.querySelector('[data-slot=dialog-content]')
    )
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
        protocol: 'openai_responses',
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
    assert.equal(draft.protocol, 'openai_responses')
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
  const dialog = required(document.querySelector('[data-slot=dialog-content]'))
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

test('mixed suppliers only suggest APIs from a concrete operation or exact published model targets', async () => {
  const { detectUpstreamApi, resolveNativeProvider } =
    await import('../lib/native-provider')
  const config: NativeConfiguration = fixture()
  const endpoint = 'https://openrouter.ai/api/v1'
  config.provider_presets = [
    {
      id: 'openrouter',
      label: 'OpenRouter',
      icon: 'OpenRouter',
      protocol: 'openai_chat',
      endpoint,
    },
    {
      id: 'custom',
      label: 'Custom',
      icon: '',
      protocol: 'openai_chat',
      endpoint: '',
    },
  ]
  const detect = (url: string, models: string[] = [], id = 999) =>
    detectUpstreamApi(config, {
      endpoint: url,
      provider: 'openrouter',
      id,
      models,
    })
  assert.deepEqual(detect(endpoint), { source: 'ambiguous' })
  assert.deepEqual(detect('https://private.invalid/v1', ['anthropic/claude']), {
    source: 'unknown',
  })
  assert.deepEqual(detect('https://openrouter.ai.example/v1'), {
    source: 'unknown',
  })
  assert.deepEqual(
    detect('https://private.invalid/v1/messages?token=private'),
    { source: 'unknown' }
  )
  assert.equal(
    resolveNativeProvider(
      config,
      undefined,
      'anthropic_messages',
      'https://private.invalid/v1'
    )?.id,
    'custom'
  )
  assert.deepEqual(detect(endpoint + '/messages'), {
    source: 'operation',
    protocol: 'anthropic_messages',
    endpoint,
  })
  assert.deepEqual(detect(endpoint + '/chat/completions'), {
    source: 'operation',
    protocol: 'openai_chat',
    endpoint,
  })
  const upstream = config.routing.upstreams[0]
  const route = config.routing.routes[0]
  const target = route.targets[0]
  config.routing.upstreams = [
    {
      ...upstream,
      id: 91,
      enabled: true,
      endpoint,
      protocol: 'openai_responses',
    },
  ]
  config.bindings.channel_providers = { '91': 'openrouter' }
  config.routing.groups = [{ name: 'group', enabled: true }]
  config.routing.routes = [
    {
      ...route,
      group: 'group',
      enabled: true,
      speeds: ['normal'],
      targets: [
        {
          ...target,
          upstream_id: 91,
          upstream_model: 'exact-model',
          weight: 1,
          speeds: ['normal'],
          enabled: true,
        },
      ],
    },
  ]
  assert.deepEqual(detect(endpoint, ['exact-model']), {
    source: 'model',
    protocol: 'openai_responses',
    endpoint,
  })
  assert.deepEqual(detect(endpoint, ['exact-model'], 91), {
    source: 'ambiguous',
  })
  assert.deepEqual(detect(endpoint, ['exact-model', 'another-model']), {
    source: 'ambiguous',
  })
  config.routing.routes[0].targets[0].enabled = false
  assert.deepEqual(detect(endpoint, ['exact-model']), { source: 'ambiguous' })
})

for (const mode of ['Create', 'Edit'] as const) {
  test(`${mode} channel applies provider protocol defaults only to new channels`, async () => {
    const config: NativeConfiguration = fixture()
    config.provider_presets = [
      {
        id: 'openai',
        label: 'OpenAI',
        icon: 'OpenAI',
        protocol: 'openai_responses',
        endpoint: 'https://api.openai.com/v1',
      },
      {
        id: 'deepseek',
        label: 'DeepSeek',
        icon: 'DeepSeek',
        protocol: 'openai_chat',
        endpoint: 'https://api.deepseek.com',
      },
    ]
    api.request = (async (options: { method: string }) => {
      assert.equal(options.method, 'GET')
      return { data: { success: true, data: config } }
    }) as typeof api.request
    const container = await mount('channels')
    await until(
      () => container.textContent?.includes('Private provider') === true
    )
    await act(async () =>
      button(container, mode === 'Create' ? 'Create Channel' : 'Edit').click()
    )
    const form = required(
      container.querySelector(`form[aria-label="${mode} Channel"]`)
    )
    const protocol = required(
      form.querySelector<HTMLSelectElement>('select[aria-label="Upstream API"]')
    )
    const provider = required(
      form.querySelector<HTMLButtonElement>('[aria-label="Upstream supplier"]')
    )
    async function choose(label: string) {
      await act(async () => provider.click())
      const option = () =>
        [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(
          (item) => item.textContent?.endsWith(label)
        )
      await until(() => !!option())
      await act(async () => required(option()).click())
    }
    if (mode === 'Create') {
      assert.equal(protocol.value, 'openai_responses')
      await choose('DeepSeek')
      assert.equal(protocol.value, 'openai_chat')
    } else {
      assert.equal(protocol.value, 'openai_chat')
    }
    await choose('OpenAI')
    assert.equal(
      protocol.value,
      mode === 'Create' ? 'openai_responses' : 'openai_chat'
    )
    const endpoint = required(
      form.querySelector<HTMLInputElement>('input[type="url"]')
    )
    const setValue = required(
      Object.getOwnPropertyDescriptor(dom.HTMLInputElement.prototype, 'value')
        ?.set
    )
    await act(async () => {
      setValue.call(endpoint, 'https://supplier.example/v1/messages')
      endpoint.dispatchEvent(
        new dom.Event('input', { bubbles: true }) as unknown as Event
      )
    })
    await act(async () => button(form, 'Use detected API and base URL').click())
    assert.equal(protocol.value, 'anthropic_messages')
    assert.equal(endpoint.value, 'https://supplier.example/v1')
    await act(async () => {
      protocol.value = 'openai_chat'
      protocol.dispatchEvent(
        new dom.Event('change', { bubbles: true }) as unknown as Event
      )
    })
    assert.equal(protocol.value, 'openai_chat')
    assert.equal(endpoint.value, 'https://supplier.example/v1')
  })
}

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
  assert.equal(button(form, 'API Keys').disabled, false)
  const models = required(
    form.querySelector<HTMLDetailsElement>('details[aria-label="Models"]')
  )
  assert.equal(models.open, false)
  assert.match(
    required(models.querySelector('summary')).textContent ?? '',
    /model-a/
  )
  assert.match(
    required(models.querySelector('summary')).textContent ?? '',
    /1 selected/
  )
  const raw = required(form.querySelector('textarea')?.closest('details'))
  const settings = required(raw.parentElement?.closest('details'))
  assert.equal(raw.open, false)
  assert.equal(settings.open, false)
  assert.ok(
    [...settings.querySelectorAll('label')].some((label) =>
      label.textContent?.includes('Tag')
    )
  )
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
  const models = required(
    form.querySelector<HTMLDetailsElement>('details[aria-label="Models"]')
  )
  await act(async () => required(models.querySelector('summary')).click())
  assert.equal(models.open, true)
  const picker = required(
    form.querySelector('section[aria-label="Available models"]')
  )
  assert.equal(button(routeFields(1), 'Remove').disabled, true)
  assert.match(
    routeFields(1).textContent ?? '',
    /Disable the channel before removing its last configured model/
  )
  await act(async () => button(picker, 'Add · team').click())
  assert.ok(routeFields(2).textContent?.includes('model-b'))
  assert.ok(routeFields(2).closest('details')?.textContent?.includes('team'))
  assert.equal(required(routeFields(2).closest('details')).open, true)
  assert.equal(required(routeFields(1).closest('details')).open, false)
  assert.equal(writes.length, 0)
  const supplier = required(routeFields(2).querySelector('details'))
  await act(async () => {
    supplier.open = false
    models.open = false
    field(routeFields(2), 'Contract revision').dispatchEvent(
      new Event('invalid', { cancelable: true })
    )
    assert.equal(form.checkValidity(), false)
  })
  assert.equal(models.open, true)
  assert.equal(supplier.open, true)
  await changeInput(field(routeFields(2), 'Upstream model'), 'supplier-b')
  await changeInput(field(routeFields(2), 'Priority'), '2')
  await changeInput(field(routeFields(2), 'Weight'), '')
  assert.ok(routeFields(2))
  assert.equal(JSON.parse(textarea.value).attachments[1].weight, '')
  await act(async () => assert.equal(form.checkValidity(), false))
  await changeInput(field(routeFields(2), 'Weight'), '7')
  await changeInput(field(routeFields(2), 'Timeout (ms)'), '15000')
  await changeInput(field(routeFields(2), 'Maximum input tokens'), '16384')
  await changeInput(field(routeFields(2), 'Maximum output tokens'), '2048')
  await changeInput(field(routeFields(2), 'Contract revision'), 'supplier-b-v1')
  assert.equal(required(routeFields(2).querySelector('details')).open, true)
  assert.equal(button(routeFields(1), 'Remove').disabled, false)
  await act(async () => button(routeFields(1), 'Remove').click())
  assert.equal(form.querySelector('[data-route-id="1"]'), null)
  assert.equal(button(routeFields(2), 'Remove').disabled, true)
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
  await changeInput(search, 'model-b')
  await act(async () => button(picker, 'Add matching bindings (2)').click())
  await changeInput(search, 'no matching model')
  assert.ok(picker.textContent?.includes('No models found'))
  await changeInput(search, 'team')
  await act(async () => button(picker, 'Add matching bindings (1)').click())
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
  await act(async () =>
    button(
      required(form.querySelector('[data-route-id="2"]')),
      'Remove'
    ).click()
  )
  const removed = JSON.parse(textarea.value)
  assert.deepEqual(
    removed.attachments.map((item: { route_id: number }) => item.route_id),
    [1, 3, 4]
  )
  assert.ok(
    removed.contexts.some(
      (item: { upstream_model: string }) => item.upstream_model === 'model-b'
    )
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
  const form = required(
    container.querySelector('form[aria-label="Edit Channel"]')
  )
  assert.ok(form.closest('[data-slot="sheet-content"]'))
  const json = required(form.querySelector('textarea'))
  assert.equal(json.closest('details')?.open, false)
  assert.equal(container.querySelector('input[type=password]'), null)
})

test('cancelling the channel editor restores its opener and keeps the filtered card expanded', async () => {
  api.request = (async (options: { method: string }) => {
    assert.equal(options.method, 'GET', 'Cancelling must not publish the draft')
    return { data: { success: true, data: fixture() } }
  }) as typeof api.request
  const container = await mount('channels')
  await until(() => !!container.querySelector('article'))
  const search = required(
    container.querySelector<HTMLInputElement>('input[aria-label="Search"]')
  )
  await changeInput(search, 'Private')
  const create = button(container, 'Create Channel')
  await act(async () => create.click())
  await act(async () =>
    button(required(container.querySelector('form')), 'Cancel').click()
  )
  assert.ok(document.activeElement === create, 'Return to Create Channel')

  const card = required(container.querySelector('article'))
  const details = required(card.querySelector('details'))
  details.open = true
  const edit = button(card, 'Edit')
  await act(async () => edit.click())
  const form = required(container.querySelector('form'))
  await changeInput(required(form.querySelector('input')), 'Unsaved name')
  await act(async () => button(form, 'Cancel').click())
  const dialog = required(document.querySelector('[data-slot=dialog-content]'))
  await act(async () => button(dialog, 'Leave').click())
  await until(() => document.activeElement === edit)
  assert.equal(container.querySelector('form'), null)
  assert.equal(details.open, true)
  assert.equal(search.value, 'Private')
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
  assert.equal(document.querySelector('[data-slot=dialog-content]'), null)
  assert.equal(
    button(container, 'Channels').getAttribute('aria-current'),
    'page'
  )

  await act(async () => button(container, 'Models').click())
  let dialog = required(document.querySelector('[data-slot=dialog-content]'))
  assert.match(dialog.textContent ?? '', /unsaved changes/)
  await act(async () => button(dialog, 'Stay').click())
  assert.equal(name.value, 'Unpublished provider')
  assert.equal(
    button(container, 'Channels').getAttribute('aria-current'),
    'page'
  )

  const create = button(container, 'Create Channel')
  await act(async () => {
    create.focus()
    create.click()
  })
  dialog = required(document.querySelector('[data-slot=dialog-content]'))
  await act(async () => button(dialog, 'Leave').click())
  assert.ok(container.querySelector('form[aria-label="Create Channel"]'))
  const form = required(
    container.querySelector('form[aria-label="Create Channel"]')
  )
  const primary = [...form.querySelectorAll('label')].filter(
    (label) => !label.closest('details')
  )
  assert.deepEqual(
    primary
      .slice(0, 4)
      .map((label) => label.querySelector('span')?.textContent),
    ['Name', 'Protocol', 'Base URL', 'API key']
  )
  assert.ok(
    form.querySelector<HTMLInputElement>('input[type=password]')?.required
  )
  assert.equal(
    required(
      form.querySelector<HTMLDetailsElement>('details[aria-label="Models"]')
    ).open,
    false
  )
  const replacementName = required(
    container.querySelector<HTMLInputElement>('form input')
  )
  assert.equal(replacementName.value, '')
  await until(() => !document.querySelector('[data-slot=dialog-content]'))
  await until(() => document.activeElement === replacementName)
  // An untouched replacement may close without another confirmation.
  await act(async () =>
    button(required(container.querySelector('form')), 'Cancel').click()
  )
  assert.equal(container.querySelector('form'), null)
  assert.equal(document.querySelector('[data-slot=dialog-content]'), null)
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
  let dialog = required(document.querySelector('[data-slot=dialog-content]'))
  await act(async () => button(dialog, 'Stay').click())
  assert.equal(textarea.value, '{unfinished')
  await act(async () =>
    button(required(container.querySelector('form')), 'Cancel').click()
  )
  dialog = required(document.querySelector('[data-slot=dialog-content]'))
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
  dialog = required(document.querySelector('[data-slot=dialog-content]'))
  await act(async () => button(dialog, 'Stay').click())
  assert.equal(secret.value, 'synthetic-unsaved-secret')
  await act(async () => button(container, 'Models').click())
  dialog = required(document.querySelector('[data-slot=dialog-content]'))
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

for (const channelCount of [1, 2]) {
  test(`route target selection reserves and releases each of ${channelCount} configured channels`, async () => {
    const config = fixture()
    if (channelCount === 2) {
      config.routing.upstreams.push({
        ...config.routing.upstreams[0],
        id: 8,
        endpoint: 'https://second.example/v1/',
      })
    }
    api.request = (async (options: { method: string }) => {
      assert.equal(options.method, 'GET')
      return { data: { success: true, data: config } }
    }) as typeof api.request
    const container = await mount('models')
    await until(() => !!container.textContent?.includes('model-a'))
    await act(async () => button(container, 'Edit').click())
    const section = required(
      container.querySelector('section[aria-label="Routing targets"]')
    )
    const add = button(section, 'Add target')
    const selections = () =>
      [...section.querySelectorAll('select')].map((select) => ({
        current: select.value,
        available: [...select.options]
          .filter((option) => !option.disabled)
          .map((option) => option.value),
      }))

    assert.deepEqual(selections(), [
      { current: '1', available: channelCount === 1 ? ['1'] : ['1', '8'] },
    ])
    if (channelCount === 2) {
      assert.equal(add.disabled, false)
      await act(async () => add.click())
      assert.deepEqual(selections(), [
        { current: '1', available: ['1'] },
        { current: '8', available: ['8'] },
      ])
    }
    assert.equal(add.disabled, true)
    await act(async () => button(section, 'Remove target').click())
    assert.equal(add.disabled, false)
    assert.deepEqual(
      selections(),
      channelCount === 1 ? [] : [{ current: '8', available: ['1', '8'] }]
    )
    await act(async () => add.click())
    assert.deepEqual(selections(), [
      ...(channelCount === 2 ? [{ current: '8', available: ['8'] }] : []),
      { current: '1', available: ['1'] },
    ])
    assert.equal(add.disabled, true)
  })
}

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

test('adding a model shows and previews the selected group tariff', async () => {
  const config = fixture()
  config.routing.groups.push(
    { name: 'FREE', enabled: true },
    { name: 'new-group', enabled: true }
  )
  config.prices.rules[0].group_multiplier_ppm = 20_000
  config.prices.rules.push({
    ...config.prices.rules[0],
    id: 2,
    group: 'FREE',
    group_multiplier_ppm: 0,
  })
  const previews: {
    route: { group: string }
    prices: typeof config.prices.rules
  }[] = []
  api.request = (async (options: {
    method: string
    url: string
    data: { model: (typeof previews)[number] }
  }) => {
    if (options.method === 'GET') {
      return { data: { success: true, data: config } }
    }
    assert.equal(options.url, '/core/v1/admin/routing/models/preview')
    previews.push(options.data.model)
    return {
      data: { success: true, data: { confirmation_digest: '1'.repeat(64) } },
    }
  }) as typeof api.request
  const container = await mount('models')
  await until(() => !!container.textContent?.includes('model-a'))
  await act(async () => button(container, 'Add Model').click())
  const form = required(container.querySelector('form'))
  const control = (name: string, tag: 'input' | 'select') =>
    required(
      [...form.querySelectorAll('label')]
        .find((label) => label.querySelector('span')?.textContent === name)
        ?.querySelector(tag)
    )
  const cost = control('Group multiplier', 'input') as HTMLInputElement
  const group = control('Group', 'select') as HTMLSelectElement
  const changeGroup = async (name: string) =>
    act(async () => {
      group.value = name
      group.dispatchEvent(new Event('change', { bubbles: true }))
    })
  assert.equal(cost.value, '0.02')
  await changeGroup('new-group')
  assert.equal(cost.value, '')
  assert.equal(cost.checkValidity(), false)
  await changeGroup('FREE')
  assert.equal(cost.value, '0')
  assert.equal(cost.checkValidity(), true)
  await changeInput(
    control('Model name', 'input') as HTMLInputElement,
    'new-model'
  )
  const displayName = control('Display name', 'input') as HTMLInputElement
  assert.equal(displayName.value, 'new-model')
  const longModel = 'x'.repeat(160)
  await changeInput(
    control('Model name', 'input') as HTMLInputElement,
    longModel
  )
  assert.equal(displayName.value, longModel.slice(0, 128))
  await changeInput(
    control('Model name', 'input') as HTMLInputElement,
    'new-model'
  )
  assert.equal(displayName.value, 'new-model')
  await changeInput(displayName, '界'.repeat(43))
  assert.equal(displayName.checkValidity(), false)
  assert.match(displayName.validationMessage, /128 UTF-8 bytes/)
  await changeInput(displayName, '\u0081')
  assert.equal(displayName.checkValidity(), false)
  await changeInput(displayName, ' ')
  assert.equal(displayName.checkValidity(), false)
  const customName = `${'界'.repeat(42)}é`
  await changeInput(displayName, customName)
  assert.equal(displayName.checkValidity(), true)
  await changeInput(
    control('Model name', 'input') as HTMLInputElement,
    'new-model-2'
  )
  assert.equal(displayName.value, customName)
  await act(async () => button(form, 'Preview').click())
  await until(() => previews.length === 1)
  assert.equal(previews[0].route.group, 'FREE')
  assert.equal(previews[0].prices[0].group_multiplier_ppm, 0)
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
  const fields = required(
    [...form.querySelectorAll('fieldset')].find(
      (item) => item.querySelector('legend')?.textContent === 'Routing target 1'
    )
  )
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
    const dialog = required(
      document.querySelector('[data-slot=dialog-content]')
    )
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
  const config = fixture()
  const route = config.routing.routes[0]
  config.routing.routes.push({
    ...route,
    id: 2,
    group: 'team',
    targets: [{ ...route.targets[0], id: 2, priority: 5 }],
  })
  api.request = (async (options: { method: string }) => {
    assert.equal(options.method, 'GET')
    return { data: { success: true, data: config } }
  }) as typeof api.request
  const container = await mount('channels')
  await until(
    () => !!container.querySelector('article[aria-label="Private provider"]')
  )
  const row = required(container.querySelector('article'))
  assert.ok(row.textContent?.includes('OpenAI Chat Completions'))
  assert.equal(required(row.querySelector('details')).open, false)
  assert.equal(button(row, 'Edit').closest('details'), null)
  assert.equal(button(row, 'Test Connection').closest('details'), null)
  assert.equal(button(row, 'API Keys').closest('details'), null)
  assert.ok(row.textContent?.includes(i18n.t('{{count}} models', { count: 1 })))
  const layout = required(row.parentElement)
  assert.equal(layout.dataset.layout, 'list')
  assert.equal(row.querySelector('dd[title="0, 5"]')?.textContent, '0, 5')
  assert.equal(row.querySelector('dd[title="1"]')?.textContent, '1')
  const checkbox = required(row.querySelector<HTMLElement>('[role=checkbox]'))
  await act(async () => checkbox.click())
  const chooseCards = required(
    container.querySelector<HTMLButtonElement>('button[aria-label="Card view"]')
  )
  await act(async () => chooseCards.click())
  assert.equal(layout.dataset.layout, 'cards')
  assert.equal(chooseCards.getAttribute('aria-pressed'), 'true')
  assert.equal(container.querySelector('article'), row)
  assert.equal(checkbox.getAttribute('aria-checked'), 'true')
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
  let dialog = required(document.querySelector('[data-slot=dialog-content]'))
  await act(async () => button(dialog, 'Stay').click())
  assert.equal(name.value, 'Unpublished channel')
  await act(async () => button(form, 'Reload editor').click())
  dialog = required(document.querySelector('[data-slot=dialog-content]'))
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
  for (const label of [
    'Disable',
    'Test Connection',
    'Fetch Models',
    'Delete Channel',
    'Edit routing',
    'API Keys',
  ]) {
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
  const dialog = required(document.querySelector('[data-slot=dialog-content]'))
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
  let dialog = required(document.querySelector('[data-slot=dialog-content]'))
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
  dialog = required(document.querySelector('[data-slot=dialog-content]'))
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

test('channel override preview preserves other channels and prices and confirms the exact version', async () => {
  const initial = fixture()
  const otherChannel = {
    headers: { 'x-client-version': 'other-channel-only' },
    parameters: {},
    status_codes: {},
  }
  let config = {
    ...initial,
    policy_sections: ['channel_overrides'],
    routing: {
      ...initial.routing,
      upstreams: [
        ...initial.routing.upstreams,
        { ...initial.routing.upstreams[0], id: 9 },
      ],
      channel_overrides: { '9': otherChannel },
    },
  }
  const changes: string[] = []
  const path = '/core/v1/admin/routing/configuration'
  api.request = (async (options: {
    url: string
    method: string
    data?: {
      expected_version: { routing: number; pricing: number }
      routing: typeof config.routing
      prices: typeof config.prices
      confirmation_digest?: string
    }
  }) => {
    if (options.method === 'GET') {
      return { data: { success: true, data: config } }
    }
    const body = required(options.data)
    changes.push(options.method)
    assert.deepEqual(body.expected_version, initial.version)
    assert.deepEqual(body.prices, initial.prices)
    assert.deepEqual(body.routing.routes, initial.routing.routes)
    assert.deepEqual(body.routing.channel_overrides[9], otherChannel)
    assert.equal('global_model_policy' in body.routing, false)
    const selected = body.routing.channel_overrides as Record<
      string,
      { parameters: Record<string, unknown> }
    >
    assert.equal(String(selected['1'].parameters.top_p), '0.85')
    const digest = 'a'.repeat(64)
    if (options.url === `${path}/preview`) {
      return {
        data: {
          success: true,
          data: {
            saved: false,
            routing: body.routing,
            expected_version: initial.version,
            confirmation_digest: digest,
          },
        },
      }
    }
    assert.equal(options.url, path)
    assert.equal(body.confirmation_digest, digest)
    config = {
      ...config,
      routing: body.routing,
      version: { routing: 2, pricing: 1 },
    }
    return { data: { success: true, data: { version: config.version } } }
  }) as typeof api.request
  const container = await mount('channels')
  await until(() => container.textContent?.includes('Override Rules') === true)
  await act(async () => button(container, 'Override Rules').click())
  await until(() => !!document.querySelector('textarea[id$="-Parameters"]'))
  const params = required(
    document.querySelector<HTMLTextAreaElement>('textarea[id$="-Parameters"]')
  )
  const set = required(
    Object.getOwnPropertyDescriptor(dom.HTMLTextAreaElement.prototype, 'value')
      ?.set
  )
  await act(async () => {
    set.call(params, '{"top_p":0.85}')
    params.dispatchEvent(
      new dom.Event('input', { bubbles: true }) as unknown as Event
    )
  })
  await act(async () => button(document.body, 'Preview changes').click())
  await until(
    () => !!document.body.textContent?.includes('Configuration preview')
  )
  assert.deepEqual(changes, ['POST'])
  assert.ok(!document.body.textContent?.includes('other-channel-only'))
  await act(async () => button(document.body, 'Confirm').click())
  await until(() => changes.includes('PUT'))
  assert.deepEqual(changes, ['POST', 'PUT'])
})

test('fetching a channel catalog previews editable changes and only Save publishes the final draft', async () => {
  const config: NativeConfiguration = fixture()
  config.routing.groups.push({ name: 'review', enabled: true })
  config.prices.rules.push({
    ...config.prices.rules[0],
    id: 3,
    group: 'review',
  })
  config.bindings.discoveries['1'] = {
    models: ['saved-only-model'],
    observed_at: 1,
  }
  config.prices.rules.push({
    ...config.prices.rules[0],
    id: 2,
    model: 'model-b',
  })
  config.routing.routes.push({
    ...config.routing.routes[0],
    id: 2,
    model: 'model-b',
    targets: [],
  })
  config.routing.routes.push({
    ...config.routing.routes[1],
    id: 3,
    group: 'review',
  })
  const page = {
    items: [
      {
        id: 1,
        lifetime: {
          complete: false,
          total_usd: null,
          exact_usd_numerator: null,
          exact_usd_denominator: null,
        },
        observation: null,
        response_time: null,
        test_time: null,
        catalog_observation: null,
      },
    ],
    total: 1,
    page: 1,
    page_size: 25,
    config_version: 1,
    pricing_version: 1,
    statistics_scope: 'channel_display_v1',
    probe_scope: 'model_catalog',
    inference_tested: false,
    snapshot_digest: 'a'.repeat(64),
    pagination_consistency: 'single_request',
    statistics_snapshot_at: 1,
    balance_units: [],
    sort_unit: null,
  }
  const writes: Array<{ method: string; url: string; data: unknown }> = []
  const previews: Array<{
    route: { model: string; group: string }
    metadata: { display_name: string }
  }> = []
  let catalogReads = 0
  let returnedModels = ['model-a', 'model-b', 'unpublished-model']
  api.request = (async (options: {
    method: string
    url: string
    data: unknown
  }) => {
    if (options.method === 'POST' && options.url.endsWith('/models/preview')) {
      previews.push(
        (options.data as { model: (typeof previews)[number] }).model
      )
      return {
        data: { success: true, data: { confirmation_digest: 'a'.repeat(64) } },
      }
    }
    if (options.method === 'PUT') {
      writes.push(options)
      config.version = { routing: 2, pricing: 1 }
      config.routing.version = 2
      page.config_version = 2
      return { data: { success: true, data: { version: config.version } } }
    }
    assert.equal(options.method, 'GET')
    if (options.url === '/api/channel/fetch_models/1') catalogReads++
    return {
      data: {
        success: true,
        data:
          options.url === '/api/channel/fetch_models/1'
            ? returnedModels
            : options.url.includes('statistics=true')
              ? page
              : config,
      },
    }
  }) as typeof api.request
  const container = await mount('channels', true, {
    ...accountingCapabilities,
    native_channel_observations: true,
  })
  await until(() =>
    [...container.querySelectorAll('button')].some(
      (item) => item.textContent === 'Edit'
    )
  )
  await act(async () => button(container, 'Edit').click())
  const editForm = required(
    container.querySelector<HTMLFormElement>('form[aria-label="Edit Channel"]')
  )
  assert.match(editForm.textContent ?? '', /saved-only-model/)
  assert.equal(catalogReads, 0)
  assert.doesNotMatch(editForm.textContent ?? '', /Remove from draft/)
  await act(async () => button(editForm, 'Cancel').click())
  await act(async () => button(container, 'Fetch Models').click())
  const form = required(
    container.querySelector<HTMLFormElement>('form[aria-label="Edit Channel"]')
  )
  await until(() => form.textContent?.includes('unpublished-model') === true)
  const picker = required(
    form.querySelector('section[aria-label="Available models"]')
  )
  assert.equal(picker.closest('details')?.open, false)
  assert.match(
    picker.closest('details')?.querySelector('summary')?.textContent ?? '',
    /Available models.*2/
  )
  assert.equal(writes.length, 0)
  assert.doesNotMatch(form.textContent ?? '', /saved-only-model/)
  assert.equal(catalogReads, 1)
  await act(async () => button(picker, 'Add · default').click())
  const models = required(
    form.querySelector('details[aria-label="Models"] summary')
  )
  assert.match(models.textContent ?? '', /model-b/)
  assert.match(models.textContent ?? '', /2 selected/)
  await act(async () => button(picker, 'Add · review').click())
  assert.match(models.textContent ?? '', /3 selected/)
  const missingSection = () =>
    [...form.querySelectorAll('details')].find((item) =>
      item.firstElementChild?.textContent?.includes(
        'Models missing from the upstream catalog'
      )
    )
  const missingRow = (name: string) =>
    required(
      [...required(missingSection()).querySelectorAll('li')].find(
        (row) => row.querySelector('span')?.textContent === name
      )
    )
  returnedModels = []
  await act(async () => button(form, 'Fetch Models').click())
  await until(() => form.textContent?.includes('Upstream catalog · 0') === true)
  assert.equal(missingSection(), undefined)

  returnedModels = ['model-a']
  await act(async () => button(form, 'Fetch Models').click())
  await until(() => !!missingSection())
  assert.equal(
    button(required(missingSection()), 'Remove from draft (2)').disabled,
    false
  )
  await act(async () =>
    button(missingRow('model-b · review'), 'Remove').click()
  )
  const remaining = JSON.parse(required(form.querySelector('textarea')).value)
  assert.deepEqual(
    remaining.attachments.map((item: { route_id: number }) => item.route_id),
    [1, 2]
  )
  assert.ok(
    remaining.contexts.some(
      (item: { upstream_model: string }) => item.upstream_model === 'model-b'
    )
  )
  assert.equal(required(missingSection()).querySelectorAll('li').length, 1)
  await act(async () =>
    button(missingRow('model-b · default'), 'Remove').click()
  )
  assert.doesNotMatch(models.textContent ?? '', /model-b/)
  assert.match(models.textContent ?? '', /1 selected/)
  assert.equal(missingSection(), undefined)

  returnedModels = ['unpublished-model']
  await act(async () => button(form, 'Fetch Models').click())
  await until(() => !!missingSection())
  assert.equal(button(missingRow('model-a · default'), 'Remove').disabled, true)
  assert.match(
    form.textContent ?? '',
    /Disable the channel before removing its last configured model/
  )
  const mapping = required(
    [...form.querySelectorAll('details')].find(
      (item) => item.firstElementChild?.textContent === 'Model Mapping'
    )
  )
  await act(async () => {
    mapping.open = true
  })
  const pick = async (label: string, text: string) => {
    const input = required(
      mapping.querySelector<HTMLInputElement>(`input[aria-label="${label}"]`)
    )
    await act(async () => input.focus())
    const option = required(
      [...mapping.querySelectorAll('[role=option]')].find(
        (item) => item.textContent === text
      )
    )
    await act(async () =>
      option.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }))
    )
  }
  await pick('Upstream model', 'unpublished-model')
  await pick('Model name', 'model-a · default')
  await act(async () => button(mapping, 'Confirm').click())
  assert.equal(missingSection(), undefined)
  assert.match(models.textContent ?? '', /model-a/)
  assert.match(models.textContent ?? '', /1 selected/)
  assert.ok(
    ![...picker.querySelectorAll('li')].some(
      (row) => row.firstElementChild?.textContent === 'unpublished-model'
    )
  )
  returnedModels = [
    'unpublished-model',
    ...Array.from({ length: 49 }, (_, index) => `catalog-${index + 1}`),
    'model-b',
    'catalog-last',
  ]
  await act(async () => button(form, 'Fetch Models').click())
  await until(() => form.textContent?.includes('Page 1 of 2') === true)
  assert.equal(form.querySelectorAll('li > span').length, 50)
  assert.equal(button(form, 'Previous page').disabled, true)
  await act(async () => button(form, 'Next page').click())
  assert.match(form.textContent ?? '', /Page 2 of 2/)
  assert.equal(picker.querySelectorAll('li > span').length, 1)
  await act(async () => button(form, 'Previous page').click())
  const search = required(
    picker.querySelector<HTMLInputElement>(
      'input[aria-label="Search models or groups"]'
    )
  )
  await changeInput(search, 'model-b')
  await act(async () => button(picker, 'Add · default').click())
  await changeInput(search, 'catalog-last')
  assert.equal(picker.querySelectorAll('li > span').length, 1)
  await act(async () => button(form, 'Fetch Models').click())
  await until(() => catalogReads === 6)
  assert.match(models.textContent ?? '', /model-b/)
  assert.equal(models.parentElement?.hasAttribute('open'), true)
  for (const row of form.querySelectorAll('fieldset[data-route-id]')) {
    const field = (name: string) =>
      required(
        [...row.querySelectorAll('label')]
          .find((label) => label.querySelector('span')?.textContent === name)
          ?.querySelector('input')
      )
    await changeInput(field('Maximum input tokens'), '16384')
    await changeInput(field('Maximum output tokens'), '2048')
    await changeInput(field('Contract revision'), 'reviewed-v2')
    if (row.getAttribute('data-route-id') === '2') {
      await changeInput(field('Weight'), '17')
    }
  }
  const nameField = required(
    [...form.querySelectorAll('label')]
      .find((item) => item.querySelector('span')?.textContent === 'Name')
      ?.querySelector('input')
  )
  await changeInput(nameField, 'Renamed channel')
  assert.equal(button(form, 'Add Model').disabled, true)
  const groupField = required(
    picker.querySelector<HTMLSelectElement>('select[aria-label="Group"]')
  )
  await act(async () => {
    groupField.value = 'review'
    groupField.dispatchEvent(new Event('change', { bubbles: true }))
  })
  await act(async () => button(form, 'Add Model').click())
  assert.ok(
    form.textContent?.includes(
      'Save this channel to continue creating catalog-last in review.'
    )
  )
  assert.equal(container.querySelector('form[aria-label="Add Model"]'), null)
  const draft = JSON.parse(required(form.querySelector('textarea')).value)
  assert.equal(draft.name, 'Renamed channel')
  assert.deepEqual(draft.expected_version, { routing: 1, pricing: 1 })
  assert.deepEqual(
    draft.attachments.map(
      (item: { upstream_model: string; weight: number }) => [
        item.upstream_model,
        item.weight,
      ]
    ),
    [
      ['unpublished-model', 1],
      ['model-b', 17],
    ]
  )
  assert.equal(writes.length, 0)
  assert.equal(form.checkValidity(), true)
  await act(async () => button(form, 'Save').click())
  await until(() => writes.length === 1)
  assert.equal(writes[0].method, 'PUT')
  assert.equal(writes[0].url, '/core/v1/admin/routing/channels/1')
  assert.deepEqual(writes[0].data, draft)
  await until(() => !!container.querySelector('form[aria-label="Add Model"]'))
  const newModel = required(
    container.querySelector('form[aria-label="Add Model"]')
  )
  const modelField = (label: string) =>
    required(
      [...newModel.querySelectorAll('label')]
        .find((item) => item.querySelector('span')?.textContent === label)
        ?.querySelector('input')
    )
  assert.equal(modelField('Model name').value, 'catalog-last')
  assert.equal(modelField('Display name').value, 'catalog-last')
  await changeInput(modelField('Maximum input tokens'), '16000')
  await changeInput(modelField('Maximum output tokens'), '2000')
  await changeInput(modelField('Contract revision'), 'supplier-v1')
  await act(async () => button(newModel, 'Preview').click())
  await until(() => previews.length === 1)
  assert.equal(previews[0].route.model, 'catalog-last')
  assert.equal(previews[0].route.group, 'review')
  assert.equal(previews[0].metadata.display_name, 'catalog-last')
  assert.equal(writes.length, 1)
})
test('each channel can save a nonempty subset of existing groups without changing routes', async () => {
  const config: NativeConfiguration = fixture()
  config.routing.groups.push({ name: 'review', enabled: true })
  const originalRoutes = structuredClone(config.routing.routes)
  const expectedVersion = { ...config.version }
  const updates: unknown[] = []
  api.request = (async (options: {
    method: string
    url: string
    data?: { patch: { groups: string[] } }
  }) => {
    const ok = (data: unknown) => ({ data: { success: true, data } })
    if (options.method === 'GET') return ok(structuredClone(config))
    assert.equal(options.method, 'PUT')
    assert.match(options.url, /channels\/batch\/configuration$/)
    updates.push(options.data)
    config.bindings.channel_groups = {
      '1': required(options.data).patch.groups,
    }
    config.version.routing++
    config.routing.version++
    return ok({ version: config.version, updated: 1, target_count: 1 })
  }) as typeof api.request
  const container = await mount('channels', true, {
    native_channel_observations: false,
  })
  await until(
    () => !!container.querySelector('article button[aria-label="Groups"]')
  )
  const trigger = required(
    container.querySelector<HTMLButtonElement>(
      'article button[aria-label="Groups"]'
    )
  )
  await act(async () => trigger.click())
  const form = required(container.querySelector('form[aria-label="Groups"]'))
  const check = (name: string) =>
    required(
      [...form.querySelectorAll<HTMLButtonElement>('[role="checkbox"]')].find(
        (input) => input.getAttribute('aria-label') === name
      )
    )
  assert.equal(check('default').getAttribute('aria-checked'), 'true')
  await act(async () => check('default').click())
  assert.ok(button(form, 'Save').disabled)
  await act(async () => check('review').click())
  await act(async () => check('default').click())
  await act(async () => button(form, 'Save').click())
  await until(() => !container.querySelector('form[aria-label="Groups"]'))
  assert.deepEqual(updates, [
    {
      expected_version: expectedVersion,
      ids: [1],
      patch: { groups: ['review', 'default'] },
    },
  ])
  assert.deepEqual(config.routing.routes, originalRoutes)
  assert.ok(container.querySelector('article')?.textContent?.includes('review'))
})

test('unpriced models can be found without opening each channel', async () => {
  const config: NativeConfiguration = fixture()
  config.bindings.discoveries['1'] = {
    models: ['catalog-only', 'pending-price'],
    observed_at: 1,
    pending_models: ['pending-price'],
  }
  api.request = (async () => ({
    data: { success: true, data: structuredClone(config) },
  })) as typeof api.request
  const container = await mount('channels', true, {
    native_channel_observations: false,
  })
  await until(() => !!container.querySelector('nav'))
  await act(async () => button(container, 'Unpriced models').click())
  assert.ok(container.textContent?.includes('pending-price'))
  assert.ok(!container.textContent?.includes('catalog-only'))
  await act(async () => button(container, 'Model onboarding').click())
  const dialog = required(container.querySelector('[role="dialog"]'))
  assert.ok(dialog.textContent?.includes('pending-price'))
})

test('channel onboarding persists missing-price selections without routing them', async () => {
  const config: NativeConfiguration = fixture()
  config.bindings.discoveries['1'] = {
    models: ['model-a'],
    observed_at: 1,
    pending_models: ['model-c'],
  }
  const originalPrices = structuredClone(config.prices)
  const originalRoutes = structuredClone(config.routing.routes)
  const writes: string[] = []
  api.request = (async (options: {
    url: string
    method: string
    data?: Record<string, unknown>
  }) => {
    const ok = (data: unknown) => ({ data: { success: true, data } })
    if (options.method === 'GET' && options.url.endsWith('/configuration')) {
      return ok(structuredClone(config))
    }
    assert.deepEqual(options.data?.expected_version, config.version)
    writes.push(options.url)
    if (options.url.endsWith('/models/synchronize')) {
      assert.equal(options.method, 'POST')
      config.bindings.discoveries[1].models = ['model-a', 'model-b']
    } else {
      assert.equal(
        options.url,
        '/core/v1/admin/routing/channels/1/models/pending'
      )
      assert.equal(options.method, 'PUT')
      const models = options.data?.models
      assert.ok(Array.isArray(models))
      assert.deepEqual([...models].sort(), ['model-b', 'model-c'])
      config.bindings.discoveries[1].pending_models = [...models]
    }
    config.version.routing++
    config.routing.version++
    return ok({
      version: { ...config.version },
      models: config.bindings.discoveries[1].models,
      routes_published: false,
    })
  }) as typeof api.request
  const page = await mount('channels', true, {
    native_channel_observations: false,
  })
  const dialog = () =>
    required(page.querySelector<HTMLElement>('[role="dialog"]'))
  const box = (name: string) =>
    page.querySelector<HTMLInputElement>(`input[aria-label="${name}"]`)
  const click = (element: HTMLElement) =>
    act(async () => {
      element.click()
    })
  await until(() => page.textContent?.includes('Model onboarding') ?? false)
  await click(button(page, 'Model onboarding'))
  await until(() => box('model-c')?.checked === true)
  assert.equal(box('model-a'), null)
  assert.ok(
    [...dialog().querySelectorAll('li')].some(
      (row) =>
        row.textContent?.includes('model-a') &&
        row.textContent.includes('Enabled')
    )
  )
  const save = () => button(dialog(), 'Save pending models')
  assert.equal(save().disabled, true)
  const hint = required(save().getAttribute('aria-describedby'))
  assert.equal(
    document.getElementById(hint)?.textContent,
    'Change the pending selection to save it.'
  )
  await click(button(dialog(), 'Sync models'))
  await until(() => !!box('model-b') && !box('model-b')?.disabled)
  assert.match(
    dialog().textContent ?? '',
    /Last sync: 2 returned, 1 newly discovered\./
  )
  assert.match(
    dialog().textContent ?? '',
    /Channel models: 1 enabled, 1 pending without an official base price\./
  )
  assert.match(
    dialog().querySelector('li')?.textContent ?? '',
    /model-b.*Newly discovered/
  )
  assert.match(
    box('model-c')?.closest('li')?.textContent ?? '',
    /Missing official base price: model-c/
  )
  assert.equal(box('model-b')?.checked, false)
  assert.equal(box('model-c')?.checked, true)
  await click(button(dialog(), 'Sync models'))
  await until(
    () =>
      dialog().textContent?.includes('2 returned, 0 newly discovered') ?? false
  )
  assert.ok(
    !dialog().querySelector('li')?.textContent?.includes('Newly discovered')
  )
  await click(required(box('model-b')))
  assert.equal(save().disabled, false)
  await click(button(dialog(), 'Save pending models'))
  await until(() => writes.length === 3 && !button(dialog(), 'Close').disabled)
  await click(button(dialog(), 'Close'))
  await until(() => !page.querySelector('[role="dialog"]'))
  await click(button(page, 'Model onboarding'))
  await until(() => box('model-b')?.checked === true)
  assert.equal(box('model-c')?.checked, true)
  assert.match(dialog().textContent ?? '', /Pending pricing/)
  assert.deepEqual(config.prices, originalPrices)
  assert.deepEqual(config.routing.routes, originalRoutes)
})
test('pending route requires a reviewed digest and leaves unrelated prices and routes alone', () =>
  checkPendingRoute('model-b'))

test('a new public model name preserves the selected upstream target and its context', () =>
  checkPendingRoute('public-model-b'))

async function checkPendingRoute(publicModel: string) {
  const config: NativeConfiguration = fixture()
  const model = 'model-b'
  config.bindings.discoveries[1] = {
    models: [model],
    observed_at: 1,
    pending_models: [model],
  }
  config.bindings.contexts.push({
    upstream_id: 1,
    upstream_model: model,
    protocol: 'openai_chat',
    endpoint: config.routing.upstreams[0].endpoint,
    max_input_tokens: 4096,
    max_output_tokens: 1024,
    contract_revision: 'test-v1',
  })
  config.bindings.official_prices = {
    models: {
      [publicModel]: {
        source: 'fixture',
        source_unit: 'USD/1M tokens',
        rates: {
          input: 100,
          output: 200,
          cache_read: 20,
          cache_write: 125,
          request: 0,
        },
      },
    },
    group_multipliers_ppm: { default: 1000000 },
    speed_multipliers_ppm: { fast: 1600000, ultrafast: 2400000 },
  }
  const before = structuredClone(config.routing.routes)
  const digest = 'a'.repeat(64)
  const mutations: string[] = []
  let preview: Record<string, unknown> | undefined
  api.request = (async (opts: {
    url: string
    method: string
    data?: Record<string, unknown>
  }) => {
    const ok = (data: unknown) => ({ data: { success: true, data } })
    if (opts.method === 'GET' && opts.url.endsWith('/configuration')) {
      return ok(structuredClone(config))
    }
    mutations.push(opts.url)
    if (opts.url.endsWith('/models/preview')) {
      assert.equal(opts.data?.editing, false)
      preview = opts.data?.model as Record<string, unknown>
      assert.deepEqual(preview.prices, [])
      assert.deepEqual(preview.expected_version, { routing: 1, pricing: 1 })
      return ok({ confirmation_digest: digest })
    }
    assert.equal(opts.method, 'POST')
    assert.equal(opts.url, '/core/v1/admin/routing/models')
    const route = required(opts.data)
      .route as NativeConfiguration['routing']['routes'][number]
    assert.deepEqual(opts.data, { ...preview, confirmation_digest: digest })
    assert.equal(route.model, publicModel)
    assert.equal(route.group, 'default')
    assert.equal(route.enabled, true)
    assert.equal(route.targets[0].upstream_id, 1)
    assert.equal(route.targets[0].upstream_model, model)
    config.routing.routes.push(route)
    config.bindings.discoveries[1].pending_models = []
    config.version = { routing: 2, pricing: 2 }
    config.routing.version = 2
    config.prices.version = 2
    return ok({ version: config.version })
  }) as typeof api.request
  const page = await mount('channels', true, {
    native_channel_observations: false,
  })
  const dialog = () =>
    required(page.querySelector<HTMLElement>('[role="dialog"]'))
  const click = (element: HTMLElement) =>
    act(async () => {
      element.click()
    })
  await until(() => page.textContent?.includes('Model onboarding') ?? false)
  await click(button(page, 'Model onboarding'))
  await until(() => !!page.querySelector('input[aria-label="model-b"]'))
  await click(button(dialog(), 'Pricing'))
  if (publicModel !== model) {
    const options = required(dialog().querySelector('details'))
    await click(required(options.querySelector('summary')))
    await changeInput(required(options.querySelector('input')), publicModel)
  }
  await until(() => dialog().textContent?.includes('Preview route') ?? false)
  assert.equal(button(dialog(), 'Publish route').disabled, true)
  await click(button(dialog(), 'Preview route'))
  await until(() => !button(dialog(), 'Publish route').disabled)
  assert.deepEqual(config.routing.routes, before)
  await click(button(dialog(), 'Publish route'))
  await until(() =>
    [...dialog().querySelectorAll('li')].some(
      (row) =>
        row.querySelector('.font-mono')?.textContent === model &&
        row.textContent?.includes('Enabled') &&
        !row.querySelector('input[type="checkbox"]')
    )
  )
  assert.deepEqual(config.routing.routes[0], before[0])
  assert.deepEqual(mutations, [
    '/core/v1/admin/routing/models/preview',
    '/core/v1/admin/routing/models',
  ])
}

for (const mapped of [false, true]) {
  test(`channel onboarding uses the ${mapped ? 'saved route' : 'selected catalog ID'} for price lookup`, async () => {
    const config: NativeConfiguration = fixture()
    const upstream = 'supplier/sku'
    const saved = 'public/saved'
    const alternative = 'public/other'
    config.bindings.discoveries[1] = {
      models: [upstream],
      pending_models: [upstream],
      observed_at: 1,
    }
    if (mapped) {
      for (const [i, name] of [alternative, saved].entries()) {
        const route = structuredClone(config.routing.routes[0])
        route.id = i + 2
        route.model = name
        Object.assign(route.targets[0], {
          id: route.id,
          upstream_model: upstream,
          enabled: false,
        })
        config.routing.routes.push(route)
      }
      config.bindings.official_prices = {
        models: {
          [saved]: {
            source: 'saved-source',
            source_unit: 'USD/token',
            rates: {
              input: 2,
              output: 3,
              cache_read: 0,
              cache_write: 0,
              request: 0,
            },
          },
        },
        group_multipliers_ppm: { default: 1000000 },
        speed_multipliers_ppm: { fast: 1000000, ultrafast: 1000000 },
      }
    }
    const previews: unknown[] = []
    api.request = (async (opts: {
      url: string
      method: string
      data?: Record<string, unknown>
    }) => {
      const ok = (data: unknown) => ({ data: { success: true, data } })
      if (opts.method === 'GET' && opts.url.endsWith('/configuration')) {
        return ok(structuredClone(config))
      }
      assert.equal(
        opts.url,
        '/core/v1/admin/routing/channels/1/models/prices/preview'
      )
      assert.equal(opts.method, 'POST')
      previews.push(opts.data)
      return ok({ channel_id: 1, model: opts.data?.model, quote: null })
    }) as typeof api.request
    const page = await mount('channels', true, {
      native_channel_observations: false,
    })
    await until(() => page.textContent?.includes('Model onboarding') === true)
    await act(async () => button(page, 'Model onboarding').click())
    const dialog = required(page.querySelector('[role="dialog"]'))
    await until(() => dialog.textContent?.includes('Pricing') === true)
    await act(async () => button(dialog, 'Pricing').click())
    const editor = () => dialog.querySelector<HTMLTextAreaElement>('textarea')
    await until(() => Boolean(editor()?.value))
    const text = required(editor()).value
    assert.ok(text.includes(mapped ? saved : upstream))
    if (mapped) assert.ok(text.includes('saved-source'))
    await act(async () => button(dialog, 'Fetch upstream prices').click())
    await until(() => previews.length === 1)
    assert.deepEqual(previews[0], {
      model: mapped ? saved : upstream,
      ...(!mapped && { upstream_model: upstream }),
    })
    assert.equal(required(editor()).value, text)
    if (mapped) {
      const selector = required(
        [...dialog.querySelectorAll('select')].find((el) =>
          [...el.options].some((option) => option.value === alternative)
        )
      )
      assert.equal(selector.value, saved)
      await act(async () => {
        selector.value = alternative
        selector.dispatchEvent(new Event('change', { bubbles: true }))
      })
      await until(() => editor()?.value.includes(alternative) === true)
      await act(async () => button(dialog, 'Fetch upstream prices').click())
      await until(() => previews.length === 2)
      assert.deepEqual(previews[1], {
        model: alternative,
      })
    }
  })
}

test('pending pricing keeps the public model name separate from the selected channel catalog entry', () =>
  checkPendingPrice())
test('a newly selected catalog entry can price a model already routed through another channel', () =>
  checkPendingPrice(2))
test('a newly selected catalog entry cannot silently quote another target on the same channel', () =>
  checkPendingPrice(1))

async function checkPendingPrice(boundChannel?: number) {
  const config: NativeConfiguration = fixture()
  const upstream = 'vendor/catalog-model'
  const alias = 'public-catalog-model'
  if (boundChannel) {
    if (boundChannel !== 1) {
      config.routing.upstreams.push({
        ...config.routing.upstreams[0],
        id: boundChannel,
      })
    }
    const route = config.routing.routes[0]
    route.model = alias
    route.targets[0].upstream_id = boundChannel
  }
  config.bindings.discoveries[1] = {
    models: [upstream],
    observed_at: 1,
    pending_models: [upstream],
  }
  let requested: Record<string, unknown> | undefined
  let reviewed: NativeConfiguration['bindings']['official_prices']
  api.request = (async (options: {
    url: string
    method: string
    data?: Record<string, unknown>
  }) => {
    const ok = (data: unknown) => ({ data: { success: true, data } })
    if (options.method === 'GET' && options.url.endsWith('/configuration')) {
      return ok(structuredClone(config))
    }
    if (options.url.endsWith('/configuration/preview')) {
      assert.equal(options.method, 'POST')
      assert.deepEqual(options.data?.expected_version, config.version)
      const bindings = required(options.data)
        .bindings as NativeConfiguration['bindings']
      reviewed = bindings.official_prices
      return ok({
        saved: false,
        bindings,
        expected_version: config.version,
        confirmation_digest: 'a'.repeat(64),
      })
    }
    assert.equal(
      options.url,
      '/core/v1/admin/routing/channels/1/models/prices/preview'
    )
    assert.equal(options.method, 'POST')
    requested = required(options.data)
    if (boundChannel === 1) {
      return ok({ channel_id: 1, model: alias, quote: null })
    }
    return ok({
      channel_id: 1,
      model: requested.model,
      quote: {
        model: requested.model,
        source: {
          kind: 'channel',
          channel_id: 1,
          upstream_model: upstream,
        },
        currency: 'USD',
        source_token_unit: 'per_token',
        token_unit: 'per_million_tokens',
        rates: {
          input: '2',
          output: '4',
          cache_read: '0',
          cache_write: '0',
          request: '0',
        },
        context_tiers: null,
      },
    })
  }) as typeof api.request
  const page = await mount('channels', true, {
    native_channel_observations: false,
  })
  const click = (element: HTMLElement) =>
    act(async () => {
      element.click()
    })
  await until(() => page.textContent?.includes('Model onboarding') ?? false)
  await click(button(page, 'Model onboarding'))
  const dialog = required(page.querySelector<HTMLElement>('[role="dialog"]'))
  await until(() => dialog.textContent?.includes('Pricing') ?? false)
  await click(button(dialog, 'Pricing'))
  const section = required(
    dialog.querySelector<HTMLElement>(`section[aria-label="${upstream}"]`)
  )
  const nameOptions = required(section.querySelector('details'))
  assert.equal(nameOptions.open, false)
  await click(required(nameOptions.querySelector('summary')))
  const input = required(nameOptions.querySelector('input'))
  await changeInput(input, alias)
  await until(
    () =>
      section
        .querySelector('textarea')
        ?.value.includes(`model = "${alias}"`) === true
  )
  const original = required(section.querySelector('textarea')).value
  await click(button(section, 'Fetch upstream prices'))
  await until(() => Boolean(requested))
  assert.deepEqual(requested, { model: alias, upstream_model: upstream })
  if (boundChannel === 1) {
    await until(
      () =>
        section.textContent?.includes('No matching upstream prices') === true
    )
    assert.equal(required(section.querySelector('textarea')).value, original)
    assert.equal(reviewed, undefined)
    return
  }
  await until(() => !button(section, 'Preview changes').disabled)
  await click(button(section, 'Preview changes'))
  await until(() => !!reviewed)
  assert.deepEqual(Object.keys(required(reviewed).models), [alias])
  assert.equal(required(reviewed).models[alias].rates.input, 2_000_000)
  assert.equal(required(reviewed).models[alias].source, 'channel/1')
  assert.deepEqual(config.bindings.discoveries[1].pending_models, [upstream])
}

for (const [protocol, meter] of [
  ['openai_responses', 'cache_write_12h'],
  ['anthropic_messages', 'cache_write_1h'],
] as const) {
  test(`pending route and official price previews keep missing ${meter} as a draft`, async () => {
    const { writeOfficialToml } =
      await import('@/features/system-settings/billing/official-price-toml')
    const config: NativeConfiguration = fixture()
    const model = 'provider-neutral-id'
    const zero = { numerator: 0, denominator: 1 }
    const paid = { numerator: 2, denominator: 1 }
    const rates = {
      input: paid,
      output: paid,
      cache_read: zero,
      cache_write: paid,
    }
    const tariff = {
      revision: protocol === 'openai_responses' ? 4 : 2,
      source_digest: '1'.repeat(64),
      uses_cache_read: true,
      uses_cache_write: true,
      [`uses_${meter}`]: true,
      tiers: [
        { up_to_input_tokens: 100, rates: { ...rates, [meter]: paid } },
        { up_to_input_tokens: null, rates: { ...rates } },
      ],
    }
    const price = {
      source: 'manual',
      source_unit: 'USD/token',
      billing_mode: 'tiered_decimal' as const,
      rates: { input: 0, output: 0, cache_read: 0, cache_write: 0, request: 0 },
      tiered: tariff,
    }
    config.routing.upstreams[0].protocol = protocol
    config.bindings.discoveries[1] = {
      models: [model],
      pending_models: [model],
      observed_at: 1,
    }
    config.bindings.contexts.push({
      upstream_id: 1,
      upstream_model: model,
      protocol,
      endpoint: config.routing.upstreams[0].endpoint,
      max_input_tokens: 4096,
      max_output_tokens: 1024,
      contract_revision: 'fixture',
    })
    config.bindings.official_prices = {
      models: { [model]: price },
      group_multipliers_ppm: { default: 1000000 },
      speed_multipliers_ppm: { fast: 1000000, ultrafast: 1000000 },
    }
    const previews: NativeConfiguration['bindings'][] = []
    api.request = (async (opts: {
      url: string
      method: string
      data?: NativeConfiguration
    }) => {
      const ok = (data: unknown) => ({ data: { success: true, data } })
      if (opts.method === 'GET' && opts.url.endsWith('/configuration')) {
        return ok(structuredClone(config))
      }
      assert.equal(opts.method, 'POST')
      assert.ok(opts.url.endsWith('/configuration/preview'))
      const bindings = required(opts.data).bindings
      previews.push(bindings)
      return ok({
        saved: false,
        bindings,
        expected_version: config.version,
        confirmation_digest: 'a'.repeat(64),
      })
    }) as typeof api.request
    const page = await mount('channels', true, {
      native_channel_observations: false,
    })
    const click = (element: HTMLElement) => act(async () => element.click())
    await until(() => page.textContent?.includes('Model onboarding') === true)
    await click(button(page, 'Model onboarding'))
    const dialog = required(page.querySelector('[role=dialog]'))
    await until(() => dialog.textContent?.includes('Pricing') === true)
    await click(button(dialog, 'Pricing'))
    const fieldset = required(dialog.querySelector('fieldset'))
    assert.equal(button(fieldset, 'Preview route').disabled, true)
    assert.match(
      required(fieldset.querySelector('[role=alert]')).textContent ?? '',
      new RegExp(meter)
    )
    const editor = required(
      dialog.querySelector('section[aria-label="Official base price"]')
    )
    await until(() => Boolean(editor.querySelector('textarea')?.value))
    const textarea = required(editor.querySelector('textarea'))
    const setText = required(
      Object.getOwnPropertyDescriptor(
        dom.HTMLTextAreaElement.prototype,
        'value'
      )?.set
    )
    async function edit(text: string) {
      await act(async () => {
        setText.call(textarea, text)
        textarea.dispatchEvent(new Event('input', { bubbles: true }))
      })
    }
    const draft = `${textarea.value}\n# pending provider confirmation\n`
    await edit(draft)
    const coverage = () =>
      required(
        [...editor.querySelectorAll<HTMLElement>('[role=status] p')].find(
          (node) => node.textContent?.includes(meter)
        )
      )
    await until(() =>
      [...editor.querySelectorAll('code')].some(
        (el) => el.textContent === `tiered.tiers[1].rates.${meter}`
      )
    )
    assert.equal(button(editor, 'Preview changes').disabled, true)
    const issue = coverage().textContent ?? ''
    assert.match(issue, /numerator = 0, denominator = 1/)
    assert.match(issue, /unknown prices in the draft/)
    assert.equal(textarea.value, draft)
    assert.equal(previews.length, 0)
    const { readFile } = await import('node:fs/promises')
    try {
      for (const locale of ['zh-CN', 'zh-TW', 'ja', 'fr', 'ru', 'vi']) {
        const messages: { translation: Record<string, string> } = JSON.parse(
          await readFile(
            new URL(`../../../i18n/locales/${locale}.json`, import.meta.url),
            'utf8'
          )
        )
        assert.ok(
          messages.translation[issue] && messages.translation[issue] !== issue
        )
        i18n.addResourceBundle(locale, 'translation', messages.translation)
        await act(async () => {
          await i18n.changeLanguage(locale)
        })
        assert.equal(coverage().textContent, messages.translation[issue])
        assert.equal(
          required(fieldset.querySelector('[role=alert]')).textContent,
          messages.translation[issue]
        )
      }
    } finally {
      await act(async () => {
        await i18n.changeLanguage('en')
      })
    }
    const corrected = {
      ...price,
      tiered: {
        ...tariff,
        tiers: [
          tariff.tiers[0],
          { up_to_input_tokens: null, rates: { ...rates, [meter]: zero } },
        ],
      },
    }
    await edit(writeOfficialToml(model, corrected))
    await until(() => !button(editor, 'Preview changes').disabled)
    await click(button(editor, 'Preview changes'))
    await until(() => previews.length === 1)
    assert.deepEqual(
      previews[0].official_prices?.models[model].tiered?.tiers[1].rates[meter],
      zero
    )
    assert.equal(Object.hasOwn(tariff.tiers[1].rates, meter), false)
  })
}

test('official price preview preserves a confirmed free scalar write across four protocols in one route group', async () => {
  const { previewOfficialPriceToml } =
    await import('@/features/system-settings/billing/official-price-api')
  const { writeOfficialToml } =
    await import('@/features/system-settings/billing/official-price-toml')
  const { readNativeRoutingScope } = await import('../native-api')
  login()
  const config: NativeConfiguration = fixture()
  const route = config.routing.routes[0]
  const protocols = [
    'openai_chat',
    'openai_responses',
    'anthropic_messages',
    'gemini',
  ]
  config.routing.upstreams = protocols.map((protocol, position) => ({
    ...config.routing.upstreams[0],
    id: position + 1,
    protocol,
  }))
  route.targets = config.routing.upstreams.map(({ id }) => ({
    ...route.targets[0],
    id,
    upstream_id: id,
  }))
  const flat = {
    source: 'manual',
    source_unit: 'USD/1M tokens',
    rates: { input: 3, output: 5, cache_read: 1, cache_write: 0, request: 0 },
  }
  const sent: string[] = []
  api.request = (async (opts: {
    method: string
    url: string
    data?: NativeConfiguration
  }) => {
    sent.push(opts.method)
    const ok = (data: unknown) => ({ data: { success: true, data } })
    if (opts.method === 'GET') {
      assert.ok(opts.url.endsWith('/configuration'))
      return ok(structuredClone(config))
    }
    assert.ok(opts.url.endsWith('/configuration/preview'))
    const bindings = required(opts.data).bindings
    assert.deepEqual(bindings.official_prices?.models[route.model], flat)
    return ok({
      saved: false,
      expected_version: config.version,
      confirmation_digest: 'b'.repeat(64),
      bindings,
    })
  }) as typeof api.request
  const seen = await previewOfficialPriceToml(
    required(readNativeRoutingScope()),
    config,
    route.model,
    writeOfficialToml(route.model, flat)
  )
  assert.deepEqual(sent, ['GET', 'POST'])
  assert.deepEqual(seen.changes.added, [route.model])
})
