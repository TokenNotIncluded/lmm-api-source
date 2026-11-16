/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { Window } from 'happy-dom'

const dom = new Window({
  url: 'https://console.example.test/usage-logs/common',
})
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'Node',
  'Element',
  'Event',
  'MutationObserver',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: dom[key],
  })
}
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
after(() => dom.close())

const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const { UsageLogsProvider, useUsageLogsContext } =
  await import('../usage-logs-provider')
const { useInitialUsageSource } =
  await import('../../lib/use-initial-usage-source')
const { readUsageStatisticsScope } = await import('../../lib/statistics')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')

const originalAdapter = api.defaults.adapter
const cleanups: Array<() => void> = []
const searchParams = {
  startTime: 1704067200000,
  endTime: 1704153600000,
  token: 'selected-key',
}
const columnFilters = [{ id: 'model_name', value: 'migrated-model' }]

type FixtureProps = {
  nativeTotal?: number
  nativeReady?: boolean
  isAdmin?: boolean
}

function Fixture({
  nativeTotal = 0,
  nativeReady = true,
  isAdmin = false,
}: FixtureProps) {
  const { usageSource, setUsageSource, initialUsageSourcePending } =
    useUsageLogsContext()
  useInitialUsageSource(
    { kind: 'common', total: nativeTotal, items: [], page: 1, page_size: 20 },
    nativeReady,
    {
      logCategory: 'common',
      historical: usageSource === 'historical',
      isAdmin,
      page: 1,
      pageSize: 20,
      searchParams,
      columnFilters,
      scope: readUsageStatisticsScope() ?? undefined,
    }
  )
  return (
    <>
      <output data-pending={initialUsageSourcePending}>{usageSource}</output>
      <button type='button' onClick={() => setUsageSource('native')}>
        Native
      </button>
      <button type='button' onClick={() => setUsageSource('historical')}>
        History
      </button>
    </>
  )
}

function login(isAdmin: boolean) {
  useAuthStore.getState().auth.setBundle({
    access_token: 'synthetic-initial-source-credential',
    token_type: 'Bearer',
    access_expires_at: 2000000000,
    user: { id: 7, username: 'synthetic', role: isAdmin ? 10 : 1 },
    session: {
      sid: 'synthetic-initial-source-session',
      current: true,
      login_method: 'password',
      ip: '127.0.0.1',
      user_agent: 'unit-only',
      created_at: 1,
      last_active_at: 1,
      expires_at: 2000000000,
    },
  })
}

function historyPage(total: number) {
  return {
    success: true,
    data: {
      source: 'legacy_log_only',
      page: 1,
      page_size: 1,
      total,
      items: total
        ? [
            {
              source_log_id: 123,
              user_id: 7,
              token_id: null,
              created_at: 1704067300,
              type: 2,
              username: 'synthetic',
              token_name: 'selected-key',
              model_name: 'migrated-model',
              quota: 1000,
              prompt_tokens: 50,
              completion_tokens: 20,
              group: 'default',
              request_id: 'legacy-request',
              quota_money_available: false,
              quota_money_basis: 'unknown',
            },
          ]
        : [],
    },
  }
}

async function pause() {
  await new Promise((resolve) => setTimeout(resolve, 10))
}
async function waitFor(condition: () => boolean) {
  for (let i = 0; i < 60 && !condition(); i += 1) await act(pause)
  assert.ok(condition())
}
async function mount(props: FixtureProps = {}) {
  login(props.isAdmin ?? false)
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  cleanups.push(() => {
    root.unmount()
    client.clear()
    container.remove()
  })
  const render = async (next: FixtureProps) => {
    await act(async () => {
      root.render(
        <QueryClientProvider client={client}>
          <UsageLogsProvider>
            <Fixture {...next} />
          </UsageLogsProvider>
        </QueryClientProvider>
      )
      await pause()
    })
  }
  await render(props)
  const source = () => container.querySelector('output')?.textContent
  const resolved = () =>
    container.querySelector('output')?.getAttribute('data-pending') === 'false'
  const select = async (label: string) => {
    const button = [...container.querySelectorAll('button')].find(
      (item) => item.textContent === label
    )
    assert.ok(button)
    await act(async () => {
      button.click()
      await pause()
    })
  }
  return { render, source, resolved, select }
}

afterEach(async () => {
  await act(async () => {
    for (const cleanup of cleanups.splice(0)) cleanup()
  })
  api.defaults.adapter = originalAdapter
  useAuthStore.getState().auth.reset('idle')
})

for (const isAdmin of [false, true]) {
  test(`${isAdmin ? 'administrator' : 'self'} empty native results select existing history with the same filters`, async () => {
    let requests = 0
    api.defaults.adapter = async (config) => {
      requests += 1
      const url = new URL(config.url ?? '', 'https://console.example.test')
      assert.equal(url.pathname, isAdmin ? '/api/log' : '/api/log/self/history')
      assert.deepEqual(Object.fromEntries(url.searchParams), {
        p: '1',
        page_size: '1',
        token_name: 'selected-key',
        start_timestamp: '1704067200',
        end_timestamp: '1704153600',
        model_name: 'migrated-model',
      })
      assert.deepEqual(config.authScope, {
        userId: 7,
        sessionId: 'synthetic-initial-source-session',
      })
      return {
        config,
        data: historyPage(12),
        headers: {},
        status: 200,
        statusText: 'OK',
      }
    }
    const view = await mount({ isAdmin })
    await waitFor(view.resolved)
    assert.equal(view.source(), 'historical')
    assert.equal(requests, 1)
    await view.select('Native')
    await view.render({ isAdmin })
    assert.equal(view.source(), 'native')
    assert.equal(
      requests,
      1,
      'manual selection does not restart automatic selection'
    )
  })
}

test('a nonempty initial native total wins even if its page has no rows', async () => {
  let requests = 0
  api.defaults.adapter = async () => {
    requests += 1
    throw new Error('History should not be requested')
  }
  const view = await mount({ nativeTotal: 2 })
  await waitFor(view.resolved)
  assert.equal(view.source(), 'native')
  await view.render({ nativeTotal: 0 })
  assert.equal(view.source(), 'native')
  assert.equal(
    requests,
    0,
    'later empty filters do not change the chosen source'
  )
})

for (const unavailable of [false, true]) {
  test(`${unavailable ? 'unavailable' : 'empty'} history keeps the native source`, async () => {
    api.defaults.adapter = async (config) => {
      if (unavailable) throw new Error('History unavailable')
      return {
        config,
        data: historyPage(0),
        headers: {},
        status: 200,
        statusText: 'OK',
      }
    }
    const view = await mount()
    await waitFor(view.resolved)
    assert.equal(view.source(), 'native')
  })
}

test('manual history selection before native completion prevents automatic selection', async () => {
  let requests = 0
  api.defaults.adapter = async () => {
    requests += 1
    throw new Error('History should not be probed')
  }
  const view = await mount({ nativeReady: false })
  assert.equal(view.source(), 'native')
  assert.equal(requests, 0)
  await view.select('History')
  await view.render({ nativeReady: true, nativeTotal: 2 })
  assert.equal(view.source(), 'historical')
  assert.equal(requests, 0)
})

test('a late history response never overrides a manual native selection', async () => {
  let release!: () => void
  const pending = new Promise<void>((resolve) => {
    release = resolve
  })
  let requested = false
  api.defaults.adapter = async (config) => {
    requested = true
    await pending
    return {
      config,
      data: historyPage(12),
      headers: {},
      status: 200,
      statusText: 'OK',
    }
  }
  const view = await mount()
  await waitFor(() => requested)
  await view.select('Native')
  await act(async () => {
    release()
    await pause()
  })
  assert.equal(view.source(), 'native')
  assert.ok(view.resolved())
})
