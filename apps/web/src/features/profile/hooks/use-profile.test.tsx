/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, test } from 'node:test'

import { Window } from 'happy-dom'

const domWindow = new Window({ url: 'https://console.example.test/profile' })
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
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { api } = await import('@/lib/api')
const { useProfile } = await import('./use-profile')
after(() => domWindow.close())

test('profile failure ends loading, keeps the actual error and recovers on explicit retry', async () => {
  const previousGet = api.get
  let state: ReturnType<typeof useProfile> | undefined
  let succeed = false
  let calls = 0
  api.get = (async (_url, config) => {
    calls++
    assert.equal(config?.skipErrorHandler, true)
    if (!succeed) throw new Error('Request failed with status code 503')
    return {
      data: { success: true, data: { id: 7, username: 'synthetic-profile' } },
    }
  }) as typeof api.get
  const root = createRoot(document.createElement('div'))
  function Harness() {
    state = useProfile()
    return null
  }
  try {
    await act(async () => root.render(<Harness />))
    assert.equal(state?.loading, false)
    assert.equal(state?.profile, null)
    assert.equal(state?.error, 'Request failed with status code 503')
    assert.equal(calls, 1)
    succeed = true
    await act(async () => {
      await state?.fetchProfile()
    })
    assert.equal(state?.error, null)
    assert.equal(state?.profile?.username, 'synthetic-profile')
    assert.equal(state?.loading, false)
    assert.equal(calls, 2)
    api.get = (async () => ({
      data: { success: false, message: 'Profile projection unavailable' },
    })) as typeof api.get
    await act(async () => {
      await state?.refreshProfile()
    })
    assert.equal(state?.profile, null)
    assert.equal(state?.error, 'Profile projection unavailable')
  } finally {
    await act(async () => root.unmount())
    api.get = previousGet
  }
})
