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
  'customElements',
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
const { SystemInfo } = await import('./index')
const { getNativeSystemResources, listSystemInstances } = await import('./api')
const { NativeResourcesPanel } =
  await import('./components/native-resources-panel')
const { SystemInstancesPanel } =
  await import('./components/system-instances-panel')
const originalGet = api.get
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })

const snapshot = {
  schema_version: 1,
  observed_at_unix_ms: 123456,
  sample_window_ms: 1000,
  system: {
    logical_cpus: 2,
    load_average: { one: 0.1, five: 0.2, fifteen: 0.3 },
    cpu_usage_percent: 5,
    memory_total_bytes: 2048,
    memory_available_bytes: 1024,
    memory_used_bytes: 1024,
    memory_usage_percent: 50,
  },
  processes: {
    'lmm-core': {
      status: 'running',
      version: '0.1.0',
      pid: 123,
      cpu_percent: 2,
      memory_bytes: 512,
      memory_percent: 25,
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
}

function respond(data: unknown) {
  api.get = (async (path) => {
    assert.equal(path, '/core/v1/admin/system-resources')
    return { data }
  }) as typeof api.get
}

let dispose: (() => Promise<void>) | undefined
afterEach(async () => {
  await dispose?.()
  dispose = undefined
  api.get = originalGet
  document.body.replaceChildren()
})
after(() => dom.close())

test('native resource response preserves real processes and accepts null samples', async () => {
  respond(snapshot)
  const actual = await getNativeSystemResources()
  assert.equal(actual.processes['lmm-core'].pid, 123)
  assert.equal(actual.processes['lmm-core'].cpu_percent, 2)
  assert.equal(actual.processes['lmm-extensions'].status, 'unavailable')
  assert.equal(actual.processes['lmm-extensions'].pid, undefined)
  assert.equal(actual.processes['lmm-extensions'].version, undefined)
})

test('native resource response rejects drift and missing process entries', async () => {
  const missingMetric = structuredClone(snapshot)
  Reflect.deleteProperty(missingMetric.processes['lmm-core'], 'pid')
  const missingProcess = structuredClone(snapshot)
  Reflect.deleteProperty(missingProcess.processes, 'lmm-extensions')
  for (const data of [
    '<html>not the endpoint</html>',
    { ...snapshot, schema_version: 2 },
    { ...snapshot, observed_at_unix_ms: 8_640_000_000_000_001 },
    { ...snapshot, system: { ...snapshot.system, memory_used_bytes: '1024' } },
    {
      ...snapshot,
      processes: {
        core: snapshot.processes['lmm-core'],
        extensions: snapshot.processes['lmm-extensions'],
      },
    },
    missingProcess,
    missingMetric,
    { ...snapshot, processes: { ...snapshot.processes, 'lmm-core': null } },
  ]) {
    respond(data)
    await assert.rejects(
      getNativeSystemResources(),
      /Invalid system resources response/
    )
  }
})

test('invalid process data renders the existing error state', async () => {
  respond({
    ...snapshot,
    processes: { core: snapshot.processes['lmm-core'] },
  })
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  dispose = async () => {
    await act(async () => root.unmount())
    client.clear()
  }
  await act(async () =>
    root.render(
      <QueryClientProvider client={client}>
        <I18nextProvider i18n={i18n}>
          <NativeResourcesPanel />
        </I18nextProvider>
      </QueryClientProvider>
    )
  )
  for (let attempt = 0; attempt < 100; attempt++) {
    if (container.textContent?.includes('Invalid system resources response')) {
      break
    }
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 10))
    })
  }
  assert.match(container.textContent ?? '', /Invalid system resources response/)
  assert.doesNotMatch(container.textContent ?? '', /Not reported/)
})

const heartbeat = {
  boot_id: 'a'.repeat(32),
  instance_name: 'worker-a',
  version: 'test-version',
  phase: 'ready',
  in_flight: 2,
  models_ready: true,
  rpc_ready: false,
  started_at: 1_700_000_000,
  heartbeat_at: 1_700_000_010,
  online: true,
}
const heartbeatList = {
  items: [heartbeat],
  truncated: false,
  stale_after_seconds: 45,
  retention_days: 0,
}
function respondInstances(data: unknown) {
  api.get = (async (path) => {
    assert.equal(path, '/core/v1/admin/instances')
    return { data }
  }) as typeof api.get
}

test('native heartbeats keep separate boot identities without inventing remote resource samples', async () => {
  respondInstances({
    ...heartbeatList,
    items: [
      heartbeat,
      { ...heartbeat, boot_id: 'b'.repeat(32), online: false },
    ],
  })
  const result = await listSystemInstances()
  assert.equal(result.data?.length, 2)
  assert.notEqual(result.data?.[0].reporter_id, result.data?.[1].reporter_id)
  assert.equal(result.data?.[0].in_flight, 2)
  assert.equal(result.data?.[1].status, 'stale')
  assert.equal(result.data?.[0].info?.resources, undefined)
  assert.equal(result.data?.[0].info?.role, undefined)
})

test('native heartbeat schema rejects legacy payloads and invalid lifecycle evidence', async () => {
  for (const value of [
    { success: true, data: [] },
    { ...heartbeatList, items: [{ ...heartbeat, online: 'yes' }] },
    { ...heartbeatList, items: [{ ...heartbeat, phase: 'unknown' }] },
  ]) {
    respondInstances(value)
    await assert.rejects(
      listSystemInstances(),
      /Invalid system instances response/
    )
  }
})

test('instance panel shows stale readiness and bounded-list notice', async () => {
  respondInstances({
    ...heartbeatList,
    truncated: true,
    items: [{ ...heartbeat, online: false }],
  })
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  dispose = async () => {
    await act(async () => root.unmount())
    client.clear()
  }
  await act(async () =>
    root.render(
      <QueryClientProvider client={client}>
        <I18nextProvider i18n={i18n}>
          <SystemInstancesPanel />
        </I18nextProvider>
      </QueryClientProvider>
    )
  )
  for (
    let attempt = 0;
    attempt < 100 && !container.textContent?.includes('worker-a');
    attempt++
  ) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 10))
    })
  }
  assert.match(container.textContent ?? '', /worker-a/)
  assert.match(container.textContent ?? '', /Stale/)
  assert.match(container.textContent ?? '', /Unavailable/)
  assert.match(container.textContent ?? '', /200 most recent/)
  assert.doesNotMatch(
    container.textContent ?? '',
    /Average CPU|master|worker-only/
  )
})

test('system info page displays native resources and instance heartbeats together', async () => {
  const requestedPaths = new Set<string>()
  api.get = (async (path) => {
    requestedPaths.add(path)
    if (path === '/core/v1/admin/system-resources') {
      return { data: snapshot }
    }
    assert.equal(path, '/core/v1/admin/instances')
    return { data: heartbeatList }
  }) as typeof api.get

  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  dispose = async () => {
    await act(async () => root.unmount())
    client.clear()
  }
  await act(async () =>
    root.render(
      <QueryClientProvider client={client}>
        <I18nextProvider i18n={i18n}>
          <SystemInfo />
        </I18nextProvider>
      </QueryClientProvider>
    )
  )
  for (let attempt = 0; attempt < 100; attempt++) {
    const content = container.textContent ?? ''
    if (content.includes('PID123') && content.includes('worker-a')) {
      break
    }
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 10))
    })
  }

  assert.match(container.querySelector('h1')?.textContent ?? '', /System Info/)
  const sections = [...container.querySelectorAll('section')]
  assert.equal(sections.length, 1)
  const row = container.querySelector('[data-instance-id]')
  assert.ok(row)
  assert.equal(row.querySelectorAll('[role="cell"]').length, 3)
  assert.equal(container.querySelectorAll('[role="columnheader"]').length, 3)
  const core = row.querySelector('[data-process-status="running"]')
  assert.match(core?.textContent ?? '', /Core process/)
  assert.match(core?.textContent ?? '', /Version: 0\.1\.0/)
  assert.match(core?.textContent ?? '', /PID123/)
  assert.match(core?.textContent ?? '', /CPU2%/)
  assert.match(core?.textContent ?? '', /RSS512 B/)
  assert.match(core?.textContent ?? '', /Memory25%/)
  assert.match(core?.textContent ?? '', /Threads4/)
  const host = row.querySelector('[data-resource="host"]')
  assert.match(host?.textContent ?? '', /Logical CPUs2/)
  assert.match(host?.textContent ?? '', /0\.1 \/ 0\.2 \/ 0\.3/)
  assert.match(host?.textContent ?? '', /Total2 KiB/)
  assert.match(host?.textContent ?? '', /Used1 KiB/)
  assert.match(host?.textContent ?? '', /Available1 KiB/)
  assert.equal(
    row.querySelector('time')?.getAttribute('datetime'),
    '1970-01-01T00:02:03.456Z'
  )
  assert.match(row.textContent ?? '', /Sample window1,000 ms/)
  assert.match(row.textContent ?? '', /RPC readinessUnavailable/)
  assert.match(row.textContent ?? '', /Extensions process/)
  assert.match(row.textContent ?? '', /worker-a/)
  assert.match(row.textContent ?? '', /test-version/)
  assert.match(row.textContent ?? '', /Online/)
  assert.deepEqual(
    requestedPaths,
    new Set(['/core/v1/admin/system-resources', '/core/v1/admin/instances'])
  )
})

for (const scenario of [
  {
    name: 'stopped boots are omitted; live and stale realtime rows remain',
    items: [
      heartbeat,
      {
        ...heartbeat,
        boot_id: 'b'.repeat(32),
        online: false,
        phase: 'stopped',
      },
      { ...heartbeat, boot_id: 'c'.repeat(32), online: false },
    ],
    truncated: false,
    linked: true,
    expectStoppedHidden: true,
  },
  {
    name: 'multiple online boots do not share local resource samples',
    items: [heartbeat, { ...heartbeat, boot_id: 'b'.repeat(32) }],
    truncated: false,
    linked: false,
    expectStoppedHidden: false,
  },
  {
    name: 'truncated heartbeat lists cannot identify a unique resource owner',
    items: [heartbeat],
    truncated: true,
    linked: false,
    expectStoppedHidden: false,
  },
]) {
  test(scenario.name, async () => {
    const requested: string[] = []
    api.get = (async (path) => {
      requested.push(path)
      return {
        data:
          path === '/core/v1/admin/system-resources'
            ? snapshot
            : {
                ...heartbeatList,
                items: scenario.items,
                truncated: scenario.truncated,
              },
      }
    }) as typeof api.get
    const container = document.createElement('div')
    document.body.append(container)
    const root = createRoot(container)
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    })
    dispose = async () => {
      await act(async () => root.unmount())
      client.clear()
    }
    await act(async () =>
      root.render(
        <QueryClientProvider client={client}>
          <I18nextProvider i18n={i18n}>
            <SystemInstancesPanel />
          </I18nextProvider>
        </QueryClientProvider>
      )
    )
    for (let attempt = 0; attempt < 100; attempt++) {
      if (
        container.querySelector('[data-instance-id]') &&
        (!scenario.linked || container.textContent?.includes('PID123'))
      ) {
        break
      }
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 10))
      })
    }
    assert.equal(
      requested.includes('/core/v1/admin/system-resources'),
      scenario.linked
    )
    assert.equal(container.textContent?.includes('PID123'), scenario.linked)
    assert.equal(container.querySelector('[data-instance-history]'), null)
    if (scenario.expectStoppedHidden) {
      assert.equal(
        container.querySelector(`[data-instance-id="${'b'.repeat(32)}"]`),
        null
      )
      const stale = container.querySelector(
        `[data-instance-id="${'c'.repeat(32)}"]`
      )
      assert.match(stale?.textContent ?? '', /Stale/)
      assert.doesNotMatch(stale?.textContent ?? '', /PID123/)
    }
  })
}

test('resource measurements distinguish measured zero, missing values and unavailable processes', async () => {
  const measured = structuredClone(snapshot)
  measured.observed_at_unix_ms = 0
  measured.system.cpu_usage_percent = 0
  measured.processes['lmm-core'].cpu_percent = 0
  measured.processes['lmm-core'].memory_bytes = 0
  measured.processes['lmm-core'].memory_percent = 0
  respond({
    ...measured,
    processes: {
      ...measured.processes,
      'lmm-core': { ...measured.processes['lmm-core'], threads: null },
    },
  })
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  dispose = async () => {
    await act(async () => root.unmount())
    client.clear()
  }
  await act(async () =>
    root.render(
      <QueryClientProvider client={client}>
        <I18nextProvider i18n={i18n}>
          <NativeResourcesPanel />
        </I18nextProvider>
      </QueryClientProvider>
    )
  )
  for (
    let attempt = 0;
    attempt < 100 && !container.querySelector('[data-process-status]');
    attempt++
  ) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 10))
    })
  }
  const running = container.querySelector('[data-process-status="running"]')
  assert.match(running?.textContent ?? '', /CPU0%/)
  assert.match(running?.textContent ?? '', /RSS0 B/)
  assert.match(running?.textContent ?? '', /Memory0%/)
  assert.match(running?.textContent ?? '', /ThreadsNot reported/)
  const unavailable = container.querySelector(
    '[data-process-status="unavailable"]'
  )
  assert.match(unavailable?.textContent ?? '', /Unavailable/)
  assert.match(unavailable?.textContent ?? '', /Not reported/)
  assert.doesNotMatch(unavailable?.textContent ?? '', /CPU|RSS|0%|0 B/)
  assert.match(container.textContent ?? '', /Sampled atNot reported/)
  assert.equal(container.querySelector('time'), null)
})
