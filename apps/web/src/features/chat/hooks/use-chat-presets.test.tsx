/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, test } from 'node:test'

import { Window } from 'happy-dom'

const dom = new Window({ url: 'https://console.example.test/chat' })
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'Node',
  'Element',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: dom[key],
  })
}
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const { api } = await import('@/lib/api')
const { useChatPresets } = await import('./use-chat-presets')
;(
  globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

after(() => dom.close())

test('chat presets follow the current flat status and do not revive cached or nested settings', async () => {
  const originalGet = api.get
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const root = createRoot(document.createElement('div'))
  let latest: ReturnType<typeof useChatPresets> | undefined
  let payload: Record<string, unknown> = {
    server_address: 'https://native.example.test',
    Chats: [{ Native: 'https://chat.example.test' }],
  }
  api.get = (async (url) => ({
    data: { success: true, data: url === '/api/status' ? payload : {} },
  })) as typeof api.get
  function Probe() {
    latest = useChatPresets()
    return null
  }
  try {
    await act(async () => {
      root.render(
        <QueryClientProvider client={client}>
          <Probe />
        </QueryClientProvider>
      )
      await new Promise((resolve) => setTimeout(resolve, 30))
    })
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 30))
    })
    assert.equal(latest?.serverAddress, 'https://native.example.test')
    assert.deepEqual(
      latest?.chatPresets.map((preset) => preset.name),
      ['Native']
    )
    payload = {
      serverAddress: 'https://old-alias.example.test',
      chats: [{ Alias: 'https://alias.example.test' }],
      data: {
        server_address: 'https://nested.example.test',
        Chats: [{ Nested: 'https://nested.example.test' }],
      },
    }
    await act(async () => {
      await latest?.retry()
      window.localStorage.setItem(
        'status',
        JSON.stringify({ Chats: [{ Cached: 'https://cached.example.test' }] })
      )
      await new Promise((resolve) => setTimeout(resolve, 30))
    })
    assert.equal(latest?.serverAddress, 'https://console.example.test')
    assert.deepEqual(latest?.chatPresets, [])
  } finally {
    await act(async () => root.unmount())
    client.clear()
    api.get = originalGet
    window.localStorage.clear()
  }
})
