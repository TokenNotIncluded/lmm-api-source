/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.
*/
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import type { AxiosAdapter, AxiosResponse } from 'axios'
import { Window } from 'happy-dom'
import type { Root } from 'react-dom/client'

import type { AuthUser } from '@/stores/auth-store'

const domWindow = new Window({ url: 'https://console.example.test/wallet' })
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'Node',
  'Element',
  'Event',
  'MutationObserver',
  'localStorage',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: domWindow[key],
  })
}

const { act, useEffect } = await import('react')
const { createRoot } = await import('react-dom/client')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { useBillingHistory } = await import('./use-billing-history')

const originalAPIAdapter = api.defaults.adapter
const reactTestGlobals = globalThis as typeof globalThis & {
  IS_REACT_ACT_ENVIRONMENT?: boolean
}
reactTestGlobals.IS_REACT_ACT_ENVIRONMENT = true

type HookValue = ReturnType<typeof useBillingHistory>
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

function billingResponse(id: number) {
  return {
    success: true,
    data: {
      items: [
        {
          id,
          user_id: 7,
          amount: id,
          money: id,
          trade_no: `order-${id}`,
          payment_method: 'stripe',
          create_time: id,
          status: 'success',
        },
      ],
      total: 1,
    },
  }
}

function Harness() {
  const value = useBillingHistory({ enabled: true })
  useEffect(() => {
    currentHook = value
  })
  return null
}

async function flushEffects() {
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
  })
}

afterEach(() => {
  api.defaults.adapter = originalAPIAdapter
  currentHook = null
  useAuthStore.getState().auth.reset('complete')
  domWindow.localStorage.clear()
})

after(() => domWindow.close())

test('sort changes reset pagination and stale responses cannot replace the new global order', async () => {
  const older = deferred<unknown>()
  const newer = deferred<unknown>()
  const secondRequestStarted = deferred<void>()
  const urls: string[] = []

  api.defaults.adapter = async (config) => {
    urls.push(String(config.url))
    const pending = urls.length === 1 ? older : newer
    if (urls.length === 2) secondRequestStarted.resolve()
    return response(config, await pending.promise)
  }

  const container = document.createElement('div')
  const root: Root = createRoot(container)
  await act(async () => root.render(<Harness />))

  try {
    await flushEffects()
    assert.equal(currentHook?.sortBy, 'create_time')
    assert.equal(currentHook?.sortOrder, 'desc')
    assert.equal(
      urls[0],
      '/api/user/topup/self?p=1&page_size=10&sort_by=create_time&sort_order=desc'
    )

    await act(async () => {
      currentHook?.handlePageChange(4)
      currentHook?.handleSortByChange('amount')
    })
    await secondRequestStarted.promise

    assert.equal(currentHook?.page, 1)
    assert.equal(currentHook?.sortBy, 'amount')
    assert.equal(
      urls[1],
      '/api/user/topup/self?p=1&page_size=10&sort_by=amount&sort_order=desc'
    )

    newer.resolve(billingResponse(20))
    await flushEffects()
    assert.equal(currentHook?.records[0]?.id, 20)

    older.resolve(billingResponse(10))
    await flushEffects()
    assert.equal(currentHook?.records[0]?.id, 20)
  } finally {
    await act(async () => root.unmount())
  }
})

test('personal wallet uses self history for root and reports failures until a successful retry', async () => {
  useAuthStore
    .getState()
    .auth.setUser({ id: 7, username: 'root-fixture', role: 100 } as AuthUser)
  const urls: string[] = []
  const originalGet = api.get
  let attempt = 0
  api.get = (async (url: string) => {
    urls.push(url)
    attempt++
    if (attempt === 1) throw new Error('503 core_migration_incomplete')
    if (attempt === 2) {
      return { data: { success: false, message: 'Unavailable' } }
    }
    return { data: { success: true, data: { items: [], total: 0 } } }
  }) as typeof api.get
  const root = createRoot(document.createElement('div'))
  try {
    await act(async () => root.render(<Harness />))
    await flushEffects()
    assert.equal(currentHook?.error, true)
    assert.equal(currentHook?.loading, false)
    await act(async () => {
      await currentHook?.refresh()
    })
    assert.equal(currentHook?.error, true)
    await act(async () => {
      await currentHook?.refresh()
    })
    assert.equal(currentHook?.error, false)
    assert.deepEqual(currentHook?.records, [])
    assert.equal(urls.length, 3)
    assert.ok(urls.every((url) => url.startsWith('/api/user/topup/self?')))
  } finally {
    api.get = originalGet
    await act(async () => root.unmount())
  }
})
