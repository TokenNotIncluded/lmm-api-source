/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { Window } from 'happy-dom'

import type { NativeConfiguration } from '../native-api'

const dom = new Window()
// Happy DOM rejects cancelled WAAPI promises; this suite tests data/navigation,
// so use Motion's non-WAAPI path while retaining the actual UI and request code.
Object.defineProperty(dom.Element.prototype, 'animate', {
  configurable: true,
  value: undefined,
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
  native_channel_observations: true,
  native_channel_usage: true,
  native_channel_counters: true,
  native_balance_probe: true,
  copy_preserve_counters: true,
}
async function mount(
  initialSection: 'channels' | 'prices',
  nativeSecrets = true,
  accounting: Partial<typeof accountingCapabilities> = accountingCapabilities,
  client = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 10_000 } },
  })
) {
  api.defaults.adapter = async (config) => {
    assert.equal(config.url, '/api/channel/ops')
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: {
        success: true,
        data: { native_secrets: nativeSecrets, ...accounting },
      },
    }
  }
  login()
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

function statistics(id: number) {
  return {
    id,
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
  }
}
function sortedPage(page = 1) {
  return {
    items: [statistics(page === 1 ? 1 : 26)],
    total: 30,
    page,
    page_size: 25,
    config_version: 1,
    pricing_version: 1,
    statistics_scope: 'channel_display_v1',
    probe_scope: 'model_catalog',
    inference_tested: false,
    snapshot_digest: 'a'.repeat(64),
    pagination_consistency: page === 1 ? 'single_request' : 'digest_checked',
    statistics_snapshot_at: 1,
    balance_units: ['CNY', 'USD'],
    sort_unit: null,
  }
}
test('native pagination carries a digest and a 409 requires a visible return to page one', async () => {
  const config = fixture()
  for (let id = 2; id <= 30; id++) {
    config.routing.upstreams.push({ ...config.routing.upstreams[0], id })
    config.bindings.names[id] = `Provider ${id}`
  }
  const requests: URL[] = []
  let changed = false
  api.request = (async (options: { url: string }) => {
    if (options.url.endsWith('/configuration')) {
      return { data: { success: true, data: config } }
    }
    const url = new URL(options.url, 'http://localhost')
    requests.push(url)
    const page = Number(url.searchParams.get('p'))
    if (page === 2 && changed) {
      throw {
        response: { status: 409, data: { code: 'CHANNEL_SNAPSHOT_CHANGED' } },
      }
    }
    return {
      data: {
        success: true,
        data: {
          ...sortedPage(page),
          pagination_consistency: url.searchParams.has('snapshot_digest')
            ? 'digest_checked'
            : 'single_request',
        },
      },
    }
  }) as typeof api.request
  const container = await mount('channels')
  await until(
    () =>
      !!container.textContent?.includes('Private provider') &&
      !button(container, 'Next').disabled
  )
  assert.equal(container.textContent?.includes('0 ms'), false)
  await act(async () => button(container, 'Next').click())
  await until(() => !!container.textContent?.includes('Provider 26'))
  assert.equal(
    requests.at(-1)?.searchParams.get('snapshot_digest'),
    'a'.repeat(64)
  )
  await act(async () => button(container, 'Previous').click())
  await until(() => !!container.textContent?.includes('Private provider'))
  changed = true
  await act(async () => button(container, 'Next').click())
  await until(
    () => !!container.textContent?.includes('channelSorting.snapshotChanged')
  )
  assert.equal(container.textContent?.includes('Provider 26'), false)
  await act(async () => button(container, 'Retry').click())
  await until(() => !!container.textContent?.includes('Private provider'))
  assert.equal(requests.at(-1)?.searchParams.get('p'), '1')
  assert.equal(requests.at(-1)?.searchParams.has('snapshot_digest'), false)
})
test('balance unit choices are explicit and never silently select or convert a currency', async () => {
  const requests: URL[] = []
  api.request = (async (options: { url: string }) => {
    if (options.url.endsWith('/configuration')) {
      return { data: { success: true, data: fixture() } }
    }
    const url = new URL(options.url, 'http://localhost')
    requests.push(url)
    return {
      data: {
        success: true,
        data: { ...sortedPage(), sort_unit: url.searchParams.get('sort_unit') },
      },
    }
  }) as typeof api.request
  const container = await mount('channels')
  await until(
    () =>
      !!container.textContent?.includes('channelSorting.balanceObservedAt') ||
      (!!container.textContent?.includes('Private provider') &&
        container.querySelector('select')?.disabled === false)
  )
  const sort = container.querySelector('select')
  assert.ok(sort)
  await act(async () => {
    sort.value = 'balance'
    sort.dispatchEvent(new Event('change', { bubbles: true }))
  })
  await until(() => container.querySelectorAll('select').length === 3)
  const unit = container.querySelectorAll('select')[2]
  assert.equal(unit.value, '')
  assert.deepEqual(
    [...unit.options].map((option) => option.value),
    ['', 'CNY', 'USD']
  )
  const count = requests.length
  await act(async () => {
    unit.value = 'USD'
    unit.dispatchEvent(new Event('change', { bubbles: true }))
  })
  await until(() => requests.length > count)
  assert.equal(requests.at(-1)?.searchParams.get('sort_unit'), 'USD')
  assert.equal(requests.at(-1)?.searchParams.has('snapshot_digest'), false)
})

async function chooseSort(container: Element, value: string) {
  const sort = container.querySelector('select')
  assert.ok(sort)
  assert.equal(sort.disabled, false)
  await act(async () => {
    sort.value = value
    sort.dispatchEvent(new Event('change', { bubbles: true }))
  })
  await until(
    () =>
      !!container.textContent?.includes('Private provider') &&
      !button(container, 'Next').disabled
  )
}
function paginatedConfiguration() {
  const config = fixture()
  for (let id = 2; id <= 30; id++) {
    config.routing.upstreams.push({ ...config.routing.upstreams[0], id })
    config.bindings.names[id] = `Provider ${id}`
  }
  return config
}
test('production 10s cache: A to B to cached A revalidates A before its uncached second page', async () => {
  const config = paginatedConfiguration()
  const requests: URL[] = []
  api.request = (async (options: { url: string }) => {
    if (options.url.endsWith('/configuration')) {
      return { data: { success: true, data: config } }
    }
    const url = new URL(options.url, 'http://localhost')
    requests.push(url)
    const page = Number(url.searchParams.get('p'))
    const digest = (
      url.searchParams.get('sort_by') === 'id' ? 'a' : 'b'
    ).repeat(64)
    return {
      data: {
        success: true,
        data: {
          ...sortedPage(page),
          snapshot_digest: digest,
          pagination_consistency: url.searchParams.has('snapshot_digest')
            ? 'digest_checked'
            : 'single_request',
        },
      },
    }
  }) as typeof api.request
  const container = await mount('channels')
  await until(
    () =>
      !!container.textContent?.includes('Private provider') &&
      !button(container, 'Next').disabled
  )
  await chooseSort(container, 'response_time')
  await chooseSort(container, 'id')
  assert.equal(
    requests.filter(
      (url) =>
        url.searchParams.get('sort_by') === 'id' &&
        url.searchParams.get('p') === '1'
    ).length,
    2
  )
  await act(async () => button(container, 'Next').click())
  await until(() => !!container.textContent?.includes('Provider 26'))
  assert.equal(requests.at(-1)?.searchParams.get('p'), '2')
  assert.equal(
    requests.at(-1)?.searchParams.get('snapshot_digest'),
    'a'.repeat(64)
  )
})
test('production 10s cache: cached continuation stays hidden until server digest validation and 409 clears it', async () => {
  const config = paginatedConfiguration()
  const requests: URL[] = []
  let changed = false
  let rejectContinuation: (() => void) | undefined
  api.request = (async (options: { url: string }) => {
    if (options.url.endsWith('/configuration')) {
      return { data: { success: true, data: config } }
    }
    const url = new URL(options.url, 'http://localhost')
    requests.push(url)
    const page = Number(url.searchParams.get('p'))
    if (changed && page === 2) {
      await new Promise<void>((_resolve, reject) => {
        rejectContinuation = () =>
          reject({
            response: {
              status: 409,
              data: { code: 'CHANNEL_SNAPSHOT_CHANGED' },
            },
          })
      })
    }
    return {
      data: {
        success: true,
        data: {
          ...sortedPage(page),
          pagination_consistency: url.searchParams.has('snapshot_digest')
            ? 'digest_checked'
            : 'single_request',
        },
      },
    }
  }) as typeof api.request
  const container = await mount('channels')
  await until(
    () =>
      !!container.textContent?.includes('Private provider') &&
      !button(container, 'Next').disabled
  )
  await act(async () => button(container, 'Next').click())
  await until(() => !!container.textContent?.includes('Provider 26'))
  await act(async () => button(container, 'Previous').click())
  await until(
    () =>
      !!container.textContent?.includes('Private provider') &&
      !button(container, 'Next').disabled
  )
  changed = true
  await act(async () => button(container, 'Next').click())
  await until(() => rejectContinuation !== undefined)
  assert.equal(
    requests.filter((url) => url.searchParams.get('p') === '2').length,
    2
  )
  assert.equal(
    requests.at(-1)?.searchParams.get('snapshot_digest'),
    'a'.repeat(64)
  )
  assert.equal(container.textContent?.includes('Provider 26'), false)
  await act(async () => rejectContinuation?.())
  await until(
    () => !!container.textContent?.includes('channelSorting.snapshotChanged')
  )
  assert.equal(container.textContent?.includes('Provider 26'), false)
})
