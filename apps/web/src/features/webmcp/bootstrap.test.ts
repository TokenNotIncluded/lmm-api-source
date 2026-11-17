/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { Window } from 'happy-dom'

import { bootstrapWebMcp } from './bootstrap'
import type { WebMcpRouter } from './tool-kit'

const router: WebMcpRouter = {
  navigate: async () => undefined,
  subscribe: () => () => undefined,
}

function withDocument(value: unknown) {
  const previous = Object.getOwnPropertyDescriptor(globalThis, 'document')
  Object.defineProperty(globalThis, 'document', {
    configurable: true,
    value,
  })
  return () => {
    if (previous) Object.defineProperty(globalThis, 'document', previous)
    else delete (globalThis as { document?: unknown }).document
  }
}

test('ordinary browsers skip the WebMCP runtime', async () => {
  for (const document of [undefined, {}, { modelContext: {} }]) {
    const restore = withDocument(document)
    try {
      assert.equal(await bootstrapWebMcp(router), undefined)
    } finally {
      restore()
    }
  }
})

test('a blocked capability getter cannot prevent startup', async () => {
  const restore = withDocument(
    Object.defineProperty({}, 'modelContext', {
      get() {
        throw new Error('Capability blocked')
      },
    })
  )
  try {
    assert.equal(await bootstrapWebMcp(router), undefined)
  } finally {
    restore()
  }
})

test('supported browsers load real tools and preserve their cleanup', async () => {
  const registered: { name: string; signal?: AbortSignal }[] = []
  let subscribed = false
  const dom = new Window({ url: 'https://lmm.test/' })
  const restore = withDocument(
    Object.assign(dom.document, {
      modelContext: {
        registerTool(
          tool: { name: string },
          options?: { signal?: AbortSignal }
        ) {
          registered.push({ name: tool.name, signal: options?.signal })
        },
      },
    })
  )
  let cleanup: (() => void) | undefined
  try {
    cleanup = await bootstrapWebMcp({
      ...router,
      subscribe: () => {
        subscribed = true
        return () => {
          subscribed = false
        }
      },
    })
    assert.equal(typeof cleanup, 'function')
    assert.ok(registered.some((tool) => tool.name === 'lmm_site_info'))
    assert.ok(registered.some((tool) => tool.name === 'lmm_settings_preview'))
    assert.equal(
      new Set(registered.map((tool) => tool.name)).size,
      registered.length
    )
    assert.ok(registered.every((tool) => tool.signal && !tool.signal.aborted))
    assert.equal(subscribed, true)
    cleanup?.()
    assert.ok(registered.every((tool) => tool.signal?.aborted))
    assert.equal(subscribed, false)
  } finally {
    cleanup?.()
    restore()
    await dom.happyDOM.abort()
  }
})
