/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, beforeEach, test } from 'node:test'

import { Window } from 'happy-dom'
import { act, createElement, useEffect } from 'react'

const dom = new Window({
  url: 'https://console.example.test/temporary-activations',
})
dom.document.write('<!doctype html><html><body></body></html>')
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'Element',
  'Node',
  'Event',
  'MutationObserver',
  'ResizeObserver',
  'getComputedStyle',
  'requestAnimationFrame',
  'cancelAnimationFrame',
  'matchMedia',
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

let readResponse: () => Promise<unknown> = async () => ({
  success: true,
  data: { purchases_available: false },
})
const requests: string[] = []

const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const { createRoot } = await import('react-dom/client')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
const previousAdapter = api.defaults.adapter
const previousAuth = useAuthStore.getState().auth
const { HeroSmsPurchaseCapabilityGate } =
  await import('./purchase-capability-gate')

let client: InstanceType<typeof QueryClient>
let root: ReturnType<typeof createRoot>
let mounted = 0
let unmounted = 0

function session(userId: number | null, sid: string | null) {
  const current = useAuthStore.getState().auth
  useAuthStore.setState({
    auth: {
      ...current,
      user:
        userId === null ? null : { id: userId, username: 'fixture', role: 1 },
      session:
        sid === null
          ? null
          : {
              sid,
              current: true,
              login_method: 'password',
              ip: null,
              user_agent: null,
              created_at: 1,
              last_active_at: 1,
              expires_at: 9999999999,
            },
    },
  })
}

function PurchaseProbe() {
  useEffect(() => {
    mounted += 1
    return () => {
      unmounted += 1
    }
  }, [])
  return createElement('p', null, 'Purchase child mounted')
}
async function flush() {
  await new Promise((resolve) => setTimeout(resolve, 0))
}
async function textAppears(value: string) {
  for (let attempt = 0; attempt < 60; attempt += 1) {
    await act(flush)
    if (document.body.textContent?.includes(value)) return
  }
  assert.fail(
    ['Expected', value, 'received', document.body.textContent].join(' ')
  )
}
async function mount() {
  await act(async () => {
    root.render(
      <QueryClientProvider client={client}>
        <I18nextProvider i18n={i18n}>
          <HeroSmsPurchaseCapabilityGate>
            <PurchaseProbe />
          </HeroSmsPurchaseCapabilityGate>
        </I18nextProvider>
      </QueryClientProvider>
    )
    await flush()
  })
}
async function refresh() {
  await act(async () => {
    await client.invalidateQueries({ queryKey: ['hero-sms-capabilities'] })
    await flush()
  })
}

beforeEach(() => {
  api.defaults.adapter = async (config) => {
    const path = config.url ?? ''
    requests.push(path)
    assert.equal(path, '/api/hero-sms/capabilities')
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: await readResponse(),
    }
  }
  document.body.innerHTML = '<div id="root"></div>'
  const element = document.getElementById('root')
  assert.ok(element)
  root = createRoot(element)
  client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: Infinity } },
  })
  mounted = 0
  unmounted = 0
  requests.length = 0
  readResponse = async () => ({
    success: true,
    data: { purchases_available: false },
  })
  session(17, 'session-a')
})
afterEach(async () => {
  await act(async () => {
    root.unmount()
    await flush()
  })
  client.clear()
  api.defaults.adapter = previousAdapter
  useAuthStore.setState({ auth: previousAuth })
})
after(async () => {
  await dom.happyDOM.cancelAsync()
})

test('pending and unsupported capabilities never mount purchase hooks', async () => {
  let resolve!: (value: unknown) => void
  readResponse = () =>
    new Promise((next) => {
      resolve = next
    })
  await mount()
  assert.equal(mounted, 0)
  assert.match(document.body.textContent || '', /Loading/)
  await act(async () => {
    resolve({ success: true, data: { purchases_available: false } })
    await flush()
  })
  await textAppears('Purchases unavailable')
  assert.equal(mounted, 0)
  assert.deepEqual(requests, ['/api/hero-sms/capabilities'])
})

for (const [name, result] of [
  ['missing capability flag', { success: true, data: {} }],
  [
    'rejected capability response',
    { success: false, data: { purchases_available: true } },
  ],
] as const) {
  test([name, 'cannot enable purchases'].join(' '), async () => {
    readResponse = async () => result
    await mount()
    await textAppears('Purchases unavailable')
    assert.equal(mounted, 0)
  })
}

for (const failure of ['revoked', 'server error'] as const) {
  test(
    [failure, 'removes an already available purchase UI'].join(' '),
    async () => {
      readResponse = async () => ({
        success: true,
        data: { purchases_available: true },
      })
      await mount()
      await textAppears('Purchase child mounted')
      assert.equal(mounted, 1)
      readResponse =
        failure === 'revoked'
          ? async () => ({
              success: true,
              data: { purchases_available: false },
            })
          : async () => {
              throw new Error('Service unavailable')
            }
      await refresh()
      await textAppears('Purchases unavailable')
      assert.equal(unmounted, 1)
    }
  )
}

test('late previous-session responses cannot enable a new session', async () => {
  let resolveOld!: (value: unknown) => void
  readResponse = () =>
    new Promise((resolve) => {
      resolveOld = resolve
    })
  await mount()
  assert.equal(mounted, 0)
  readResponse = async () => ({
    success: true,
    data: { purchases_available: false },
  })
  await act(async () => {
    session(17, 'session-b')
    await flush()
  })
  await textAppears('Purchases unavailable')
  await act(async () => {
    resolveOld({ success: true, data: { purchases_available: true } })
    await flush()
  })
  assert.equal(mounted, 0)
  assert.match(document.body.textContent || '', /Purchases unavailable/)
  assert.equal(requests.length, 2)
})

test('signing out removes a cached purchase capability without another read', async () => {
  readResponse = async () => ({
    success: true,
    data: { purchases_available: true },
  })
  await mount()
  await textAppears('Purchase child mounted')
  await act(async () => {
    session(null, null)
    await flush()
  })
  await textAppears('Purchases unavailable')
  assert.equal(unmounted, 1)
  assert.equal(requests.length, 1)
})
