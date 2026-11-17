/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */

import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { Window } from 'happy-dom'

const dom = new Window({ url: 'http://localhost/' })
const matchMedia = dom.matchMedia.bind(dom)
dom.matchMedia = (query: string) => {
  const media = matchMedia(query)
  if (query === '(prefers-reduced-motion)') {
    Object.defineProperty(media, 'matches', { value: true })
  }
  return media
}
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLButtonElement',
  'Element',
  'Node',
  'Event',
  'MouseEvent',
  'MutationObserver',
  'getComputedStyle',
  'requestAnimationFrame',
  'cancelAnimationFrame',
  'scrollTo',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: dom[key],
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
const { createRootRoute, createRouter, createMemoryHistory, RouterProvider } =
  await import('@tanstack/react-router')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { SummaryCards } = await import('./summary-cards')
const { AdminSiteStatisticsPanel } = await import('./admin-site-statistics')
const { ApiInfoPanel } = await import('./api-info-panel')
const { DEFAULT_CURRENCY_CONFIG, useSystemConfigStore } =
  await import('@/stores/system-config-store')
const { PerformanceHealthPanel } = await import('./performance-health-panel')
const { NativeResourcesPanel } =
  await import('@/features/system-info/components/native-resources-panel')
const { UptimePanel } = await import('./uptime-panel')
const { PanelWrapper } = await import('../ui/panel-wrapper')
const originalGet = api.get
const originalPut = api.put
const originalConfig = useSystemConfigStore.getState().config
const originalUser = useAuthStore.getState().auth.user
const i18n = createInstance()
await i18n.use(initReactI18next).init({
  lng: 'en',
  resources: { en: { translation: { credits: 'Credits' } } },
})
let dispose: (() => Promise<void>) | undefined

afterEach(async () => {
  await dispose?.()
  dispose = undefined
  api.get = originalGet
  api.put = originalPut
  useSystemConfigStore.setState({ config: originalConfig })
  useAuthStore.getState().auth.setUser(originalUser)
  document.body.replaceChildren()
})
after(() => dom.close())

async function mount(Component: () => React.ReactNode) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const route = createRootRoute({ component: Component })
  const router = createRouter({
    routeTree: route,
    history: createMemoryHistory({ initialEntries: ['/'] }),
  })
  await router.load()
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  await act(async () =>
    root.render(
      <QueryClientProvider client={client}>
        <I18nextProvider i18n={i18n}>
          <RouterProvider router={router} />
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
  for (let i = 0; i < 100; i++) {
    if (check()) return
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 10))
    })
  }
  assert.ok(check(), 'UI did not reach expected state')
}
function button(container: HTMLElement, label: string) {
  const found = [...container.querySelectorAll('button')].find(
    (el) => el.textContent === label
  )
  assert.ok(found, `Missing ${label} button`)
  return found
}

test('native resource failures show the Core reason and allow retry', async () => {
  api.get = (async () => {
    throw Object.assign(new Error('Request failed with status code 503'), {
      response: { data: { error: { message: 'Resource sample unavailable' } } },
    })
  }) as typeof api.get
  const container = await mount(NativeResourcesPanel)
  await until(() =>
    Boolean(container.textContent?.includes('Resource sample unavailable'))
  )
  assert.ok(button(container, 'Retry'))
})

test('native resources use service process keys and leave missing samples unknown', async () => {
  api.get = (async () => ({
    data: {
      schema_version: 1,
      observed_at_unix_ms: Date.now(),
      sample_window_ms: 250,
      system: {
        logical_cpus: 4,
        load_average: { one: 0.1, five: 0.2, fifteen: 0.3 },
        cpu_usage_percent: 23,
        memory_usage_percent: 50,
        memory_total_bytes: 2097152,
        memory_available_bytes: 1048576,
        memory_used_bytes: 1048576,
      },
      processes: {
        'lmm-core': {
          status: 'running',
          version: 'local-fixture',
          pid: 123,
          cpu_percent: 2,
          memory_bytes: 1048576,
          memory_percent: 50,
          threads: 4,
        },
        'lmm-extensions': {
          status: 'unavailable',
          version: null,
          pid: null,
          cpu_percent: null,
          memory_bytes: null,
          memory_percent: null,
          threads: null,
        },
      },
    },
  })) as typeof api.get
  const container = await mount(NativeResourcesPanel)
  await until(() => Boolean(container.textContent?.includes('123')))
  assert.match(container.textContent ?? '', /23%/)
  assert.match(container.textContent ?? '', /2%/)
  const extensions = [...container.querySelectorAll('div')].find(
    (element) => element.firstElementChild?.textContent === 'Extensions process'
  )
  assert.ok(extensions)
  assert.match(extensions.textContent ?? '', /Not reported/)
  assert.doesNotMatch(extensions.textContent ?? '', /0%|0 MB/)
})

test('empty public API info does not claim model routing is unconfigured', async () => {
  api.get = (async () => ({
    data: { success: true, data: { api_info: [] } },
  })) as typeof api.get
  const container = await mount(ApiInfoPanel)
  await until(() =>
    Boolean(container.textContent?.includes('No data available'))
  )
  assert.doesNotMatch(container.textContent ?? '', /No API routes configured/)
})

test('performance distinguishes failed responses from empty results and retries', async () => {
  let calls = 0
  api.get = (async () => ({
    data:
      ++calls === 1
        ? { success: false, data: { models: [] } }
        : { success: true, data: { models: [] } },
  })) as typeof api.get
  const container = await mount(PerformanceHealthPanel)
  await until(
    () => container.textContent?.includes('Failed to load data') === true
  )
  assert.doesNotMatch(container.textContent ?? '', /No data available/)
  await act(async () => button(container, 'Retry').click())
  await until(
    () => container.textContent?.includes('No data available') === true
  )
  assert.doesNotMatch(
    container.textContent ?? '',
    /Failed to load data|Success rate/
  )
  assert.equal(container.querySelector('.animate-pulse'), null)
  await act(async () => button(container, 'Refresh').click())
  await until(() => calls === 3)
})

test('performance shows the Core failure reason without claiming no traffic', async () => {
  api.get = (async () => {
    throw Object.assign(new Error('Request failed with status code 503'), {
      response: {
        data: {
          error: {
            code: 'core_migration_incomplete',
            message: 'Performance history is unavailable',
          },
        },
      },
    })
  }) as typeof api.get
  const container = await mount(PerformanceHealthPanel)
  await until(() =>
    Boolean(
      container.textContent?.includes('Performance history is unavailable')
    )
  )
  assert.doesNotMatch(
    container.textContent ?? '',
    /No data available|Success rate/
  )
  assert.ok(button(container, 'Retry'))
})

for (const [label, response] of [
  ['failed envelope', { success: false, data: [] }],
  ['missing data', { success: true }],
  ['null data', { success: true, data: null }],
  ['non-array data', { success: true, data: {} }],
] as const) {
  test(`uptime rejects ${label} without claiming monitoring is unconfigured`, async () => {
    let calls = 0
    api.get = (async () => ({
      data: ++calls === 1 ? response : { success: true, data: [] },
    })) as typeof api.get
    const container = await mount(UptimePanel)
    await until(() => container.querySelector('[role="alert"]') !== null)
    assert.match(container.textContent ?? '', /Failed to load/)
    assert.doesNotMatch(
      container.textContent ?? '',
      /No uptime monitoring configured/
    )
    await act(async () => button(container, 'Retry').click())
    await until(
      () =>
        container.textContent?.includes('No uptime monitoring configured') ===
        true
    )
    assert.equal(container.querySelector('[role="alert"]'), null)
    assert.equal(calls, 2)
  })
}

test('uptime refresh failure hides stale health and permits recovery', async () => {
  const groups = [
    {
      categoryName: 'Local services',
      monitors: [{ name: 'Core API', status: 1, uptime: 1 }],
    },
  ]
  let calls = 0
  api.get = (async () => {
    if (++calls === 2) throw new Error('503 unavailable')
    return { data: { success: true, data: groups } }
  }) as typeof api.get
  const container = await mount(UptimePanel)
  await until(() => container.textContent?.includes('Core API') === true)
  assert.match(container.textContent ?? '', /100\.00%/)
  const refresh = container.querySelector<HTMLButtonElement>(
    'button[aria-label="Refresh"]'
  )
  assert.ok(refresh)
  await act(async () => refresh.click())
  await until(() => container.querySelector('[role="alert"]') !== null)
  assert.doesNotMatch(
    container.textContent ?? '',
    /Core API|100\.00%|No uptime monitoring configured/
  )
  await act(async () => button(container, 'Retry').click())
  await until(() => container.textContent?.includes('Core API') === true)
  assert.equal(container.querySelector('[role="alert"]'), null)
  assert.equal(calls, 3)
})

test('uptime initial transport failure is not a successful empty configuration', async () => {
  api.get = (async () => {
    throw new Error('503 unavailable')
  }) as typeof api.get
  const container = await mount(UptimePanel)
  await until(() => container.querySelector('[role="alert"]') !== null)
  assert.doesNotMatch(
    container.textContent ?? '',
    /No uptime monitoring configured/
  )
  assert.ok(button(container, 'Retry'))
})

test('traffic list orders by request count, not server array position', async () => {
  api.get = (async () => ({
    data: {
      success: true,
      data: {
        models: [
          {
            model_name: 'quiet-model',
            request_count: 1,
            avg_latency_ms: 100,
            avg_tps: 20,
            success_rate: 1,
          },
          {
            model_name: 'busy-model',
            request_count: 100,
            avg_latency_ms: 200,
            avg_tps: 30,
            success_rate: 1,
          },
        ],
      },
    },
  })) as typeof api.get
  const container = await mount(PerformanceHealthPanel)
  await until(() => container.textContent?.includes('busy-model') === true)
  const text = container.textContent ?? ''
  assert.ok(text.indexOf('busy-model') < text.indexOf('quiet-model'))
})

test('failed usage stays unknown and a new account fetches its own usage', async () => {
  type User = NonNullable<typeof originalUser>
  const user = {
    id: 901,
    username: 'qa',
    wallet_unit: 'core_credit',
    financial_projection_available: true,
    usage_projection_available: true,
    role: 1,
    quota: 5000000,
    used_quota: 1000,
    request_count: 3,
    group: 'default',
  } as User
  useAuthStore.getState().auth.setUser(user)
  let usageCalls = 0
  api.get = (async (url: string) => {
    if (url === '/api/status') {
      return {
        data: {
          success: true,
          data: { system_name: 'Test', display_in_currency: true },
        },
      }
    }
    if (url === '/api/data/self') {
      usageCalls++
      return { data: { success: usageCalls > 1, data: [] } }
    }
    throw new Error(`Unexpected ${url}`)
  }) as typeof api.get
  const container = await mount(SummaryCards)
  await until(
    () => container.textContent?.includes('Failed to load data') === true
  )
  assert.doesNotMatch(container.textContent ?? '', /Healthy|No recent usage/)
  assert.equal(
    container.querySelector('[data-metric="todayUsage"] .overview-metric-value')
      ?.textContent,
    '—'
  )
  assert.match(container.textContent ?? '', /1,000 Raw credits/)
  assert.match(container.textContent ?? '', /Original imported counters/)
  await act(async () =>
    useAuthStore.getState().auth.setUser({ ...user, id: 902 })
  )
  await until(
    () =>
      usageCalls === 2 &&
      container.textContent?.includes('0 Raw credits') === true
  )
  assert.doesNotMatch(container.textContent ?? '', /Failed to load data/)
})

test('successful null usage renders an empty overview without crashing', async () => {
  type User = NonNullable<typeof originalUser>
  useAuthStore.getState().auth.setUser({
    id: 903,
    username: 'empty-usage',
    wallet_unit: 'core_credit',
    financial_projection_available: true,
    usage_projection_available: true,
    role: 1,
    quota: 5000000,
    used_quota: 0,
    request_count: 0,
    group: 'default',
  } as User)
  api.get = (async (url: string) => {
    if (url === '/api/status') {
      return {
        data: {
          success: true,
          data: { system_name: 'Test', display_in_currency: true },
        },
      }
    }
    if (url === '/api/data/self') {
      return { data: { success: true, data: null } }
    }
    throw new Error(`Unexpected ${url}`)
  }) as typeof api.get
  const container = await mount(SummaryCards)
  await until(() => container.textContent?.includes('0 Raw credits') === true)
  assert.match(container.textContent ?? '', /Usage at a glance/)
  assert.match(container.textContent ?? '', /Last 24h usage/)
  assert.doesNotMatch(
    container.textContent ?? '',
    /Failed to load data|No recent usage/
  )
})

test('missing consumption is not turned into zero when request rows exist', async () => {
  configureCurrencyOverview()
  const originalRead = api.get
  api.get = (async (
    url: string,
    config: { params?: { start_timestamp?: number } } = {}
  ) =>
    url === '/api/data/self'
      ? {
          data: {
            success: true,
            data: [
              {
                created_at: config.params?.start_timestamp,
                count: 3,
                quota: null,
              },
            ],
          },
        }
      : originalRead(url, config)) as typeof api.get
  const container = await mount(SummaryCards)
  await until(
    () =>
      container.textContent?.includes(
        'Usage totals are incomplete for this period.'
      ) === true
  )
  assert.equal(
    container.querySelector('[data-metric="todayUsage"] .overview-metric-value')
      ?.textContent,
    '—'
  )
  assert.doesNotMatch(
    container.querySelector('[data-metric="todayUsage"]')?.textContent ?? '',
    /0 Raw credits/
  )
  assert.match(
    container.querySelector('[data-metric="usage"]')?.textContent ?? '',
    /1,000,000 Credits/
  )
})

test('empty and loading panels preserve their header action', async () => {
  let clicks = 0
  const container = await mount(() => (
    <>
      <PanelWrapper
        title='Empty'
        empty
        headerActions={
          <button type='button' onClick={() => clicks++}>
            Refresh empty
          </button>
        }
      />
      <PanelWrapper
        title='Loading'
        loading
        headerActions={
          <button type='button' onClick={() => clicks++}>
            Refresh loading
          </button>
        }
      />
    </>
  ))
  await act(async () => {
    button(container, 'Refresh empty').click()
    button(container, 'Refresh loading').click()
  })
  assert.equal(clicks, 2)
})

function configureCurrencyOverview() {
  const currency = {
    ...DEFAULT_CURRENCY_CONFIG,
    currencyUnit: 'credit' as const,
    creditsPerUsd: 500000,
    creditsPerUsdExact: '500000',
    cnyPerUsd: 7,
    cnyPerUsdExact: '7',
  }
  useSystemConfigStore.getState().setConfig({ currency })
  useSystemConfigStore.getState().setLoading(false)
  useAuthStore.getState().auth.setUser({
    id: 905,
    username: 'review',
    wallet_unit: 'core_credit',
    financial_projection_available: true,
    role: 10,
    quota: 5000000,
    used_quota: 1000000,
    normalized_used_quota: 1000000,
    usage_projection_available: true,
    request_count: 20,
    setting: {
      wallet_display_currency: 'USD',
      settlement_currency: 'CNY',
      language: 'en',
    },
  })
  api.get = (async (
    url: string,
    config?: { params?: { start_timestamp?: number } }
  ) => {
    if (url === '/api/finance/site-statistics') {
      return {
        data: {
          success: true,
          data: {
            as_of: 1791504000,
            credits_per_usd: 500000,
            total_used_credits: '9277415232383220730',
            total_balance_credits: '500000000',
            recharge: {
              currencies: [
                {
                  currency: 'USD',
                  gross_amount_micros: '3000000',
                  refunded_amount_micros: '250000',
                  net_amount_micros: '2750000',
                  orders: 1,
                },
              ],
              virtual_units: [],
              confirmed_orders: 1,
              unconfirmed_orders: 0,
              invalid_orders: 0,
            },
          },
        },
      }
    }
    if (url === '/api/status') {
      return {
        data: {
          success: true,
          data: {
            system_name: 'Review',
            currency_unit: 'credit',
            credits_per_usd: 500000,
            cny_per_usd:
              useSystemConfigStore.getState().config.currency.cnyPerUsd,
          },
        },
      }
    }
    if (url === '/api/data/self') {
      const createdAt: unknown = config?.params?.start_timestamp
      assert.equal(typeof createdAt, 'number')
      return {
        data: {
          success: true,
          data: [{ created_at: createdAt, quota: 250000 }],
        },
      }
    }
    throw new Error(`Unexpected ${url}`)
  }) as typeof api.get
}

for (const shape of ['nonzero', 'zero', 'absent'] as const) {
  test(`financial projection false keeps the ${shape} wallet unknown while real usage stays visible`, async () => {
    configureCurrencyOverview()
    const current = useAuthStore.getState().auth.user
    assert.ok(current)
    useAuthStore.getState().auth.setUser({
      ...current,
      financial_projection_available: false,
      quota: shape === 'nonzero' ? 5_000_000 : shape === 'zero' ? 0 : undefined,
      request_count:
        shape === 'nonzero' ? 20 : shape === 'zero' ? 0 : undefined,
    })
    const container = await mount(SummaryCards)
    await until(
      () =>
        container
          .querySelector('.overview-balance-value')
          ?.textContent?.trim() === 'Unknown'
    )
    assert.equal(container.querySelector('.overview-health'), null)
    assert.match(
      container.querySelector('.overview-balance-footer')?.textContent ?? '',
      /The current wallet balance could not be read/
    )
    const metricValue = (title: string) =>
      [...container.querySelectorAll('.overview-metric')]
        .find((row) => row.querySelector('dt')?.textContent === title)
        ?.querySelector('.overview-metric-value')?.textContent
    await until(() => metricValue('Last 24h usage') === '250,000 Raw credits')
    assert.equal(metricValue('Recorded consumption'), '1,000,000 Credits')
    assert.equal(
      metricValue('Request Count'),
      shape === 'nonzero' ? '20' : shape === 'zero' ? '0' : '—'
    )
    assert.doesNotMatch(
      container.querySelector('.overview-balance')?.textContent ?? '',
      /Balance depleted|Healthy|10 USD/
    )
  })
}

for (const financial of [undefined, true]) {
  test(`native balance requires explicit financial facts (${financial}) and missing usage stays unavailable`, async () => {
    configureCurrencyOverview()
    const user = useAuthStore.getState().auth.user
    assert.ok(user)
    useAuthStore.getState().auth.setUser({
      ...user,
      financial_projection_available: financial,
      usage_projection_available: false,
      normalized_used_quota: null,
    })
    const container = await mount(SummaryCards)
    assert.equal(
      container.querySelector('.overview-balance-value')?.textContent?.trim(),
      financial === true ? '5,000,000 Credits' : 'Unknown'
    )
    const metrics =
      container.querySelector('.overview-metrics')?.textContent ?? ''
    assert.equal(
      container.querySelector('[data-metric="usage"] .overview-metric-value')
        ?.textContent,
      '—'
    )
    assert.equal(
      container.querySelector('[data-metric="requests"] .overview-metric-value')
        ?.textContent,
      '—'
    )
    assert.doesNotMatch(metrics, /1,000,000|20/)
    assert.equal(container.querySelector('[data-currency]'), null)
  })
}

test('native wallet with unknown imported history stays raw and keeps reservations separate', async () => {
  configureCurrencyOverview()
  const current = useAuthStore.getState().auth.user
  assert.ok(current)
  useAuthStore.getState().auth.setUser({
    ...current,
    quota: 199671,
    financial_projection_available: true,
    wallet_unit: 'core_credit',
    wallet_money_basis: 'unknown',
    wallet_money_available: false,
    wallet_spendable_quota: 199671,
    wallet_reserved_quota: 280,
    wallet_debt: 0,
    usage_projection_available: false,
    used_quota: undefined,
    normalized_used_quota: undefined,
    request_count: undefined,
  })
  const container = await mount(SummaryCards)
  assert.match(
    container.querySelector('.overview-balance-value')?.textContent ?? '',
    /199,671 Credits/
  )
  assert.match(
    container.querySelector('[data-native-wallet-details]')?.textContent ?? '',
    /280 Credits/
  )
  assert.equal(container.querySelector('[data-currency="USD"]'), null)
  const metrics =
    container.querySelector('.overview-metrics')?.textContent ?? ''
  assert.match(metrics, /—/)
  assert.doesNotMatch(
    container.querySelector('.overview-balance')?.textContent ?? '',
    /Healthy|No recent usage/
  )
})

function CurrencyOverview() {
  return (
    <>
      <SummaryCards />
      <AdminSiteStatisticsPanel />
    </>
  )
}

test('native summary stays in credits across preferences while site cash and unpriced history keep their units', async () => {
  configureCurrencyOverview()
  const user = useAuthStore.getState().auth.user
  assert.ok(user)
  const writes: unknown[] = []
  api.put = (async (_url, body) => {
    writes.push(body)
    return { data: { success: true } }
  }) as typeof api.put
  let payments: string | null | undefined
  for (const [currency, siteBalance] of [
    ['USD', '1,000 USD'],
    ['CNY', '7,000 CNY'],
    ['CREDIT', '500,000,000 Credits'],
  ] as const) {
    useAuthStore.getState().auth.setUser({
      ...user,
      setting: {
        wallet_display_currency: currency,
        settlement_currency: 'CNY',
        language: 'en',
      },
    })
    const container = await mount(CurrencyOverview)
    await until(
      () => container.querySelectorAll('.overview-payment-row').length === 1
    )
    assert.match(
      container.querySelector('.overview-balance-value')?.textContent ?? '',
      /5,000,000 Credits/
    )
    assert.match(
      container.querySelector('.overview-metrics')?.textContent ?? '',
      /2,000,000 Credits|1,000,000 Credits/
    )
    assert.match(
      container.querySelector('.overview-metrics')?.textContent ?? '',
      /250,000 Raw credits/
    )
    assert.match(
      container.querySelector('.overview-site-totals')?.textContent ?? '',
      new RegExp(siteBalance)
    )
    assert.match(
      container.querySelector('.overview-balance-footer')?.textContent ?? '',
      /Current personal wallet/
    )
    assert.equal(container.querySelector('.overview-health'), null)
    assert.equal(container.querySelector('[data-currency]'), null)
    const currentPayments =
      container.querySelector('.overview-payments')?.textContent
    if (payments !== undefined) assert.equal(currentPayments, payments)
    payments = currentPayments
    if (currency === 'CREDIT') {
      assert.match(
        container.querySelector('.overview-site-value')?.textContent ?? '',
        /9,277,415,232,383,220,730 Credits/
      )
    }
    await dispose?.()
    dispose = undefined
  }
  assert.deepEqual(writes, [])
})

test('a depleted current balance stays known when historical quota cannot be valued', async () => {
  configureCurrencyOverview()
  const user = useAuthStore.getState().auth.user
  assert.ok(user)
  for (const quota of [0, -1]) {
    useAuthStore.getState().auth.setUser({ ...user, quota })
    const container = await mount(SummaryCards)
    await until(
      () =>
        container
          .querySelector('.overview-metrics')
          ?.textContent?.includes('250,000 Raw credits') === true
    )
    assert.match(
      container.querySelector('.overview-health')?.textContent ?? '',
      /Balance depleted/
    )
    assert.match(
      container.querySelector('.overview-balance-footer')?.textContent ?? '',
      /Current personal wallet/
    )
    assert.doesNotMatch(
      container.querySelector('.overview-balance-footer')?.textContent ?? '',
      /Unknown/
    )
    await dispose?.()
    dispose = undefined
  }
})

test('missing FX cannot turn site balances into zero or alter cash payments', async () => {
  configureCurrencyOverview()
  useSystemConfigStore.getState().setConfig({
    currency: {
      ...useSystemConfigStore.getState().config.currency,
      cnyPerUsd: 0,
    },
  })
  const user = useAuthStore.getState().auth.user
  assert.ok(user)
  useAuthStore
    .getState()
    .auth.setUser({ ...user, setting: { wallet_display_currency: 'CNY' } })
  api.put = (async () => ({ data: { success: true } })) as typeof api.put
  const container = await mount(CurrencyOverview)
  await until(
    () => container.querySelectorAll('.overview-site-value').length === 2
  )

  assert.equal(
    container.querySelector('.overview-balance-value')?.textContent,
    '5,000,000 Credits'
  )
  assert.ok(
    [...container.querySelectorAll('.overview-site-value')].every(
      (value) => value.textContent === '- CNY'
    )
  )
  assert.match(
    container.querySelector('.overview-payments')?.textContent ?? '',
    /2.75 USD/
  )
  assert.match(
    container.querySelector('.overview-exact-credits')?.textContent ?? '',
    /9,277,415,232,383,220,730/
  )
})
