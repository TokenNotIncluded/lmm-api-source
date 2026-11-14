/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.
*/
import assert from 'node:assert/strict'
import { after, afterEach, describe, test } from 'node:test'

import type { AxiosAdapter, AxiosResponse } from 'axios'
import { Window } from 'happy-dom'
import type { Root } from 'react-dom/client'

const domWindow = new Window({ url: 'https://console.example.test/models' })
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
    value: domWindow[key],
  })
}

const { act, useEffect, useLayoutEffect } = await import('react')
const { createRoot } = await import('react-dom/client')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const deploymentSettingsModule = await import('./use-model-deployment-settings')
const { clearConnectionCache, useModelDeploymentSettings } =
  deploymentSettingsModule

const originalAPIAdapter = api.defaults.adapter
const reactTestGlobals = globalThis as typeof globalThis & {
  IS_REACT_ACT_ENVIRONMENT?: boolean
}
reactTestGlobals.IS_REACT_ACT_ENVIRONMENT = true

type HookValue = ReturnType<typeof useModelDeploymentSettings>
let currentHook: HookValue | null = null

function response(
  config: Parameters<AxiosAdapter>[0],
  data: unknown
): AxiosResponse {
  return {
    config,
    data,
    headers: {},
    status: 200,
    statusText: 'OK',
  }
}

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise
  })
  return { promise, resolve }
}

type OnCommit = (owner: number | undefined, value: HookValue) => void
function Harness({ onCommit }: { onCommit?: OnCommit }) {
  const owner = useAuthStore((state) => state.auth.user?.id)
  const value = useModelDeploymentSettings()
  useLayoutEffect(() => {
    onCommit?.(owner, value)
  }, [owner, value, onCommit])
  useEffect(() => {
    currentHook = value
  })
  return null
}

async function mountHarness(onCommit?: OnCommit): Promise<{
  root: Root
  container: HTMLDivElement
}> {
  const container = document.createElement('div')
  const root = createRoot(container)
  await act(async () => {
    root.render(<Harness onCommit={onCommit} />)
  })
  return { root, container }
}

async function flushEffects() {
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
  })
}

afterEach(() => {
  api.defaults.adapter = originalAPIAdapter
  clearConnectionCache()
  currentHook = null
  useAuthStore.getState().auth.reset()
})

after(() => domWindow.close())

describe('useModelDeploymentSettings', () => {
  test('ignores a stale settings response after a newer refresh completes', async () => {
    const older = deferred<unknown>()
    const newer = deferred<unknown>()
    const secondRequestStarted = deferred<void>()
    let settingsRequests = 0
    let connectionRequests = 0

    api.defaults.adapter = async (config) => {
      if (config.url === '/api/deployments/settings') {
        settingsRequests += 1
        if (settingsRequests === 2) secondRequestStarted.resolve()
        const pending = settingsRequests === 1 ? older : newer
        const data = await pending.promise
        return response(config, data)
      }
      if (config.url === '/api/deployments/settings/test-connection') {
        connectionRequests += 1
        return response(config, { success: true })
      }
      throw new Error(`unexpected request: ${String(config.url)}`)
    }

    const { root } = await mountHarness()

    try {
      await flushEffects()
      assert.equal(settingsRequests, 1)

      await act(async () => {
        void currentHook?.refresh()
      })
      await secondRequestStarted.promise
      assert.equal(settingsRequests, 2)

      newer.resolve({ success: true, data: { enabled: false } })
      await flushEffects()
      assert.equal(currentHook?.isIoNetEnabled, false)
      assert.equal(currentHook?.loading, false)
      assert.equal(currentHook?.loadingPhase, 'done')

      older.resolve({ success: true, data: { enabled: true } })
      await flushEffects()

      assert.equal(currentHook?.isIoNetEnabled, false)
      assert.equal(currentHook?.loading, false)
      assert.equal(currentHook?.loadingPhase, 'done')
      assert.equal(connectionRequests, 0)
    } finally {
      await act(async () => root.unmount())
    }
  })
  test('every committed account transition starts isolated, including A to B to A', async () => {
    const second = deferred<unknown>()
    const third = deferred<unknown>()
    let requests = 0
    let connections = 0
    const commits: {
      owner: number | undefined
      enabled: boolean
      ok: boolean | null
      loading: boolean
    }[] = []
    useAuthStore
      .getState()
      .auth.setUser({ id: 101, username: 'fixture-a', role: 10 })
    api.defaults.adapter = async (config) => {
      if (config.url === '/api/deployments/settings') {
        requests++
        const data =
          requests === 1
            ? { success: true, data: { enabled: true, revision: 'a' } }
            : await (requests === 2 ? second : third).promise
        return response(config, data)
      }
      assert.equal(config.url, '/api/deployments/settings/test-connection')
      connections++
      return response(config, { success: true })
    }
    const { root } = await mountHarness((owner, value) => {
      commits.push({
        owner,
        enabled: value.isIoNetEnabled,
        ok: value.connectionOk,
        loading: value.loading,
      })
    })
    try {
      await flushEffects()
      assert.equal(currentHook?.isIoNetEnabled, true)
      assert.equal(currentHook?.connectionOk, true)
      await act(async () => {
        useAuthStore
          .getState()
          .auth.setUser({ id: 202, username: 'fixture-b', role: 10 })
      })
      const b = commits.filter((value) => value.owner === 202)
      assert.ok(b.length > 0)
      assert.ok(
        b.every((value) => !value.enabled && value.ok === null && value.loading)
      )
      second.resolve({ success: true, data: { enabled: false, revision: 'b' } })
      await flushEffects()
      assert.equal(currentHook?.loading, false)
      const beforeReturn = commits.length
      await act(async () => {
        useAuthStore
          .getState()
          .auth.setUser({ id: 101, username: 'fixture-a', role: 10 })
      })
      const returned = commits.slice(beforeReturn)
      assert.ok(returned.length > 0)
      assert.ok(
        returned.every(
          (value) =>
            value.owner === 101 &&
            !value.enabled &&
            value.ok === null &&
            value.loading
        )
      )
      third.resolve({ success: true, data: { enabled: false, revision: 'a' } })
      await flushEffects()
      assert.equal(currentHook?.loading, false)
      assert.equal(currentHook?.isIoNetEnabled, false)
      assert.equal(requests, 3)
      assert.equal(connections, 1)
    } finally {
      await act(async () => root.unmount())
    }
  })

  test('focus reuses the revision cache but manual connection testing bypasses it', async () => {
    const focusSettings = deferred<unknown>()
    const manualConnection = deferred<unknown>()
    let requests = 0
    let connections = 0
    api.defaults.adapter = async (config) => {
      if (config.url === '/api/deployments/settings') {
        requests++
        return response(
          config,
          requests === 2
            ? await focusSettings.promise
            : { success: true, data: { enabled: true, revision: 'stable' } }
        )
      }
      assert.equal(config.url, '/api/deployments/settings/test-connection')
      connections++
      return response(
        config,
        connections === 1 ? { success: true } : await manualConnection.promise
      )
    }
    const { root } = await mountHarness()
    try {
      await flushEffects()
      assert.equal(currentHook?.connectionOk, true)
      await act(async () => {
        window.dispatchEvent(new Event('focus'))
      })
      assert.equal(currentHook?.loading, true)
      assert.equal(currentHook?.loadingPhase, 'settings')
      focusSettings.resolve({
        success: true,
        data: { enabled: true, revision: 'stable' },
      })
      await flushEffects()
      assert.equal(currentHook?.loading, false)
      assert.equal(connections, 1)
      await act(async () => {
        void currentHook?.testConnection()
      })
      assert.equal(currentHook?.connectionLoading, true)
      assert.equal(currentHook?.loadingPhase, 'connection')
      manualConnection.resolve({ success: false, message: 'Connection failed' })
      await flushEffects()
      assert.equal(currentHook?.connectionOk, false)
      assert.equal(currentHook?.connectionError, 'Connection failed')
      assert.equal(currentHook?.loading, false)
      assert.equal(requests, 3)
      assert.equal(connections, 2)
    } finally {
      await act(async () => root.unmount())
    }
  })
})
