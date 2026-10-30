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
  retention_days: 7,
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
    if (content.includes('Core process') && content.includes('worker-a')) {
      break
    }
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 10))
    })
  }

  assert.match(container.querySelector('h1')?.textContent ?? '', /System Info/)
  const sections = [...container.querySelectorAll('section')]
  const resources = sections.find((section) =>
    section
      .querySelector('h3')
      ?.textContent?.includes('Service resource breakdown')
  )
  const instances = sections.find(
    (section) => section.querySelector('h3')?.textContent === 'Instances'
  )
  assert.ok(resources)
  assert.ok(instances)
  assert.match(resources.textContent ?? '', /Core process.*PID123/)
  assert.match(resources.textContent ?? '', /Extensions process/)
  assert.match(instances.textContent ?? '', /worker-a/)
  assert.match(instances.textContent ?? '', /test-version/)
  assert.match(instances.textContent ?? '', /Online/)
  assert.deepEqual(
    requestedPaths,
    new Set(['/core/v1/admin/system-resources', '/core/v1/admin/instances'])
  )
})
