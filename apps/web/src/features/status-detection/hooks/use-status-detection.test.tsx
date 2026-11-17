/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, test } from 'node:test'

import { Window } from 'happy-dom'

const dom = new Window({ url: 'https://synthetic.example/status' })
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'Node',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: dom[key],
  })
}
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { useStatusDetection } = await import('./use-status-detection')
const { bindAuthCache, clearAuthentication } =
  await import('@/lib/auth-session')
const originalGet = api.get
const originalAuth = useAuthStore.getState().auth
const flush = () => new Promise((resolve) => setTimeout(resolve, 30))
after(() => {
  api.get = originalGet
  useAuthStore.setState({ auth: originalAuth })
  dom.close()
})

test('anonymous checks skip private metrics and account switches use separate performance caches', async () => {
  useAuthStore.getState().auth.reset('complete')
  const requests: string[] = []
  api.get = (async (url: string) => {
    requests.push(url)
    const model = `model-${useAuthStore.getState().auth.user?.id}`
    return {
      status: 200,
      data:
        url === '/health/ready'
          ? {
              service: 'lmm-core',
              status: 'ready',
              database: true,
              capabilities: { identity: true, models: true },
              readiness_scope: 'configured_components',
              upstream_health_checked: false,
            }
          : {
              success: true,
              data: url.endsWith('/summary')
                ? { models: [{ model_name: model }] }
                : { model_name: model, groups: [] },
            },
    }
  }) as typeof api.get
  const client = new QueryClient()
  let latest!: ReturnType<typeof useStatusDetection>
  function Probe() {
    latest = useStatusDetection()
    return null
  }
  const root = createRoot(document.createElement('div'))
  try {
    await act(async () => {
      root.render(
        <QueryClientProvider client={client}>
          <Probe />
        </QueryClientProvider>
      )
      await flush()
    })
    await act(flush)
    assert.equal(latest.requiresSignIn, true)
    assert.equal(latest.health?.status, 'ready')
    assert.deepEqual(requests, ['/health/ready'])
    await act(async () => {
      await latest.refresh()
    })
    assert.deepEqual(requests, ['/health/ready', '/health/ready'])

    for (const id of [51, 52]) {
      await act(async () => {
        useAuthStore.getState().auth.setBundle({
          user: { id, username: `synthetic-${id}`, role: 1 },
          access_token: `synthetic-token-${id}`,
          token_type: 'Bearer',
          access_expires_at: 9999999999,
          session: {
            sid: `synthetic-session-${id}`,
            current: true,
            login_method: 'password',
            ip: null,
            user_agent: null,
            created_at: 1,
            last_active_at: 1,
            expires_at: 9999999999,
          },
        })
        await flush()
      })
      await act(flush)
      assert.equal(latest.requiresSignIn, false)
      assert.deepEqual(latest.availableModels, [`model-${id}`])
    }
    assert.equal(requests.filter((url) => url.endsWith('/summary')).length, 2)
  } finally {
    await act(async () => root.unmount())
    client.clear()
  }
})

test('clearing expired authentication preserves the pending public health check', async () => {
  useAuthStore.getState().auth.reset('complete')
  const client = new QueryClient()
  const unbind = bindAuthCache(client)
  let resolve!: (value: unknown) => void
  const pending = new Promise((done) => {
    resolve = done
  })
  api.get = (async (url: string) => {
    assert.equal(url, '/health/ready')
    return pending
  }) as typeof api.get
  let latest!: ReturnType<typeof useStatusDetection>
  function Probe() {
    latest = useStatusDetection()
    return null
  }
  const root = createRoot(document.createElement('div'))
  try {
    await act(async () => {
      root.render(
        <QueryClientProvider client={client}>
          <Probe />
        </QueryClientProvider>
      )
      await flush()
    })
    assert.equal(latest.healthLoading, true)
    client.setQueryData(['perf-metrics-summary', '51:old-session', 24], {
      private: true,
    })
    await act(async () => {
      clearAuthentication(false)
    })
    assert.equal(
      client.getQueryData(['perf-metrics-summary', '51:old-session', 24]),
      undefined
    )
    await act(async () => {
      resolve({
        status: 200,
        data: {
          service: 'lmm-core',
          status: 'ready',
          database: true,
          capabilities: { identity: true, models: false },
          readiness_scope: 'configured_components',
          upstream_health_checked: false,
        },
      })
      await flush()
    })
    assert.equal(latest.healthLoading, false)
    assert.equal(latest.health?.status, 'ready')
    assert.equal(latest.requiresSignIn, true)
  } finally {
    await act(async () => root.unmount())
    unbind()
    client.clear()
  }
})
