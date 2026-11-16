/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import { test } from 'node:test'

import axios from 'axios'

type CookieModule = typeof import('./auth-cookie')
const cookieRequire = createRequire(import.meta.url)
const KEY = 'new-api:auth-cookie:uncertain'
const marker = 'pending:synthetic-crashed-writer'

function realm(
  backing = new Map<string, string>(),
  cache = new Map(backing),
  onListen?: () => void
) {
  const descriptors = new Map(
    ['window', 'navigator', 'isSecureContext'].map((key) => [
      key,
      Object.getOwnPropertyDescriptor(globalThis, key),
    ])
  )
  const events = new EventTarget()
  const listeners = new Set<EventListenerOrEventListenerObject>()
  const timers = new Map<number, () => void>()
  let nextTimer = 0,
    held = 0,
    readFails = false
  Object.defineProperty(globalThis, 'isSecureContext', {
    configurable: true,
    value: true,
  })
  Object.defineProperty(globalThis, 'navigator', {
    configurable: true,
    value: {
      locks: {
        async request<T>(
          _name: string,
          _options: unknown,
          callback: () => Promise<T>
        ) {
          held++
          try {
            return await callback()
          } finally {
            held--
          }
        },
      },
    },
  })
  Object.defineProperty(globalThis, 'window', {
    configurable: true,
    value: {
      localStorage: {
        getItem(key: string) {
          if (readFails) throw new Error('Storage read denied')
          return cache.get(key) ?? null
        },
        setItem(key: string, value: string) {
          cache.set(key, value)
          backing.set(key, value)
        },
        removeItem(key: string) {
          cache.delete(key)
          backing.delete(key)
        },
      },
      addEventListener(
        type: string,
        listener: EventListenerOrEventListenerObject
      ) {
        assert.equal(held, 1, 'handoff must wait inside the acquired WebLock')
        listeners.add(listener)
        events.addEventListener(type, listener)
        onListen?.()
      },
      removeEventListener(
        type: string,
        listener: EventListenerOrEventListenerObject
      ) {
        listeners.delete(listener)
        events.removeEventListener(type, listener)
      },
      setTimeout(callback: () => void, delay: number) {
        assert.equal(delay, 10_000)
        const id = ++nextTimer
        timers.set(id, callback)
        return id
      },
      clearTimeout(id: number) {
        timers.delete(id)
      },
    },
  })
  // Bun's ESM URL queries share a cache entry. Reload the actual module to
  // model separate realm-local blocked state without a product reset hook.
  const path = cookieRequire.resolve('./auth-cookie.ts')
  delete cookieRequire.cache[path]
  const cookie = cookieRequire(path) as CookieModule
  return {
    cookie,
    cache,
    backing,
    listeners,
    timers,
    failReads() {
      readFails = true
    },
    notice(key: string | null, newValue: string | null = null) {
      const event = new Event('storage')
      Object.defineProperties(event, {
        key: { value: key },
        newValue: { value: newValue },
      })
      events.dispatchEvent(event)
    },
    expire() {
      const callbacks = Array.from(timers.values())
      timers.clear()
      for (const callback of callbacks) callback()
    },
    close() {
      try {
        assert.equal(held, 0)
        assert.equal(listeners.size, 0)
        assert.equal(timers.size, 0)
      } finally {
        for (const [key, descriptor] of descriptors) {
          if (descriptor) Object.defineProperty(globalThis, key, descriptor)
          else Reflect.deleteProperty(globalThis, key)
        }
      }
    },
  }
}

function deferred() {
  let resolve!: () => void
  const promise = new Promise<void>((done) => {
    resolve = done
  })
  return { promise, resolve }
}

test('a successor realm waits for peer deletion delayed past the WebLock handoff', async () => {
  const shared = new Map<string, string>()
  const writer = realm(shared)
  const entered = deferred(),
    release = deferred()
  let staleCache = new Map<string, string>()
  const previous = writer.cookie.withAuthCookieLock(async (transaction) =>
    transaction.request(async () => {
      staleCache = new Map(shared)
      entered.resolve()
      await release.promise
    })
  )
  await entered.promise
  assert.match(staleCache.get(KEY) ?? '', /^pending:/)
  release.resolve()
  await previous
  assert.equal(shared.has(KEY), false)
  writer.close()

  const reader = realm(shared, staleCache)
  let attempts = 0
  try {
    const next = reader.cookie.withAuthCookieLock(async (transaction) =>
      transaction.request(async () => {
        attempts++
      })
    )
    assert.equal(attempts, 0)
    assert.equal(reader.listeners.size, 1)
    assert.equal(reader.cookie.getAuthCookieBlockedState(), null)
    reader.notice(KEY)
    assert.equal(
      attempts,
      0,
      'a deletion event alone cannot prove cache convergence'
    )
    reader.cache.delete(KEY)
    reader.notice(KEY)
    await next
    assert.equal(attempts, 1)
    assert.equal(reader.cookie.getAuthCookieBlockedState(), null)
    assert.equal(shared.has(KEY), false)
  } finally {
    reader.close()
  }
})

test('registration immediately rereads storage to close a missed deletion race', async () => {
  const cache = new Map([[KEY, marker]])
  const reader = realm(new Map(), cache, () => cache.delete(KEY))
  let attempts = 0
  try {
    await reader.cookie.withAuthCookieLock(async () => {
      attempts++
    })
    assert.equal(attempts, 1)
    assert.equal(reader.cookie.getAuthCookieBlockedState(), null)
  } finally {
    reader.close()
  }
})

test('storage clear requires a verified absent marker and unrelated events do not release', async () => {
  const reader = realm(new Map([[KEY, marker]]))
  let attempts = 0
  try {
    const next = reader.cookie.withAuthCookieLock(async () => {
      attempts++
    })
    reader.notice('unrelated')
    reader.notice(null)
    assert.equal(attempts, 0)
    reader.cache.clear()
    reader.backing.clear()
    reader.notice(null)
    await next
    assert.equal(attempts, 1)
  } finally {
    reader.close()
  }
})

test('a crashed writer becomes durable uncertainty that explicit recovery can reconcile', async () => {
  const reader = realm(new Map([[KEY, marker]]))
  let attempts = 0
  try {
    const rejected = assert.rejects(
      reader.cookie.withAuthCookieLock(async (transaction) =>
        transaction.request(async () => {
          attempts++
        })
      ),
      reader.cookie.AuthCookieBlockedError
    )
    assert.equal(reader.cookie.getAuthCookieBlockedState(), null)
    assert.equal(reader.timers.size, 1)
    reader.expire()
    await rejected
    const uncertainMarker = 'uncertain:synthetic-crashed-writer'
    assert.equal(reader.backing.get(KEY), uncertainMarker)
    assert.equal(
      reader.cookie.getAuthCookieUncertaintyMarker(),
      uncertainMarker
    )
    assert.equal(reader.cookie.getAuthCookieBlockedState(), 'uncertain')
    assert.equal(
      await reader.cookie.withAuthCookieReconciliationLock(
        uncertainMarker,
        async () => 'confirmation-required'
      ),
      'confirmation-required'
    )
    assert.equal(reader.backing.get(KEY), uncertainMarker)
    reader.cache.clear()
    reader.notice(null)
    await assert.rejects(
      reader.cookie.withAuthCookieLock(async () => {
        attempts++
      }),
      reader.cookie.AuthCookieBlockedError
    )
    assert.equal(attempts, 0)
  } finally {
    reader.close()
  }
})

test('existing uncertainty rejects immediately and memory uncertainty never self-recovers', async () => {
  const reader = realm(new Map([[KEY, 'uncertain:lost-response']]))
  let attempts = 0
  try {
    await assert.rejects(
      reader.cookie.withAuthCookieLock(async () => {
        attempts++
      }),
      reader.cookie.AuthCookieBlockedError
    )
    assert.equal(reader.timers.size, 0)
    reader.cache.clear()
    reader.backing.clear()
    await assert.rejects(
      reader.cookie.withAuthCookieLock(async () => {
        attempts++
      }),
      reader.cookie.AuthCookieBlockedError
    )
    assert.equal(attempts, 0)
    assert.equal(reader.cookie.getAuthCookieBlockedState(), 'uncertain')
  } finally {
    reader.close()
  }
})

test('an uncertain event rejects even if a later clear is already visible', async () => {
  const reader = realm(new Map([[KEY, marker]]))
  let attempts = 0
  try {
    const rejected = assert.rejects(
      reader.cookie.withAuthCookieLock(async () => {
        attempts++
      }),
      reader.cookie.AuthCookieBlockedError
    )
    reader.cache.clear()
    reader.notice(KEY, 'uncertain:peer-response-lost')
    await rejected
    assert.equal(reader.cookie.getAuthCookieBlockedState(), 'uncertain')
    assert.equal(attempts, 0)
  } finally {
    reader.close()
  }
})

test('abort cleans listeners and timers without poisoning the reader or sending', async () => {
  const reader = realm(new Map([[KEY, marker]]))
  const controller = new AbortController()
  const abortListeners = new Set<EventListenerOrEventListenerObject>()
  const add = controller.signal.addEventListener.bind(controller.signal)
  const remove = controller.signal.removeEventListener.bind(controller.signal)
  controller.signal.addEventListener = (
    type: string,
    listener: EventListenerOrEventListenerObject | null,
    options?: AddEventListenerOptions | boolean
  ) => {
    if (listener) {
      abortListeners.add(listener)
      add(type, listener, options)
    }
  }
  controller.signal.removeEventListener = (
    type: string,
    listener: EventListenerOrEventListenerObject | null,
    options?: EventListenerOptions | boolean
  ) => {
    if (listener) {
      abortListeners.delete(listener)
      remove(type, listener, options)
    }
  }
  let attempts = 0
  try {
    const rejected = assert.rejects(
      reader.cookie.withAuthCookieLock(async () => {
        attempts++
      }, controller.signal),
      (error: unknown) => axios.isCancel(error)
    )
    assert.equal(abortListeners.size, 1)
    controller.abort()
    await rejected
    assert.equal(abortListeners.size, 0)
    assert.equal(reader.backing.get(KEY), marker)
    assert.equal(reader.cookie.getAuthCookieBlockedState(), null)
    reader.cache.delete(KEY)
    reader.backing.delete(KEY)
    await reader.cookie.withAuthCookieLock(async () => {
      attempts++
    })
    assert.equal(attempts, 1)
  } finally {
    reader.close()
  }
})

test('abort after marker convergence still prevents the operation from entering', async () => {
  const reader = realm(new Map([[KEY, marker]]))
  const controller = new AbortController()
  let attempts = 0
  try {
    const rejected = assert.rejects(
      reader.cookie.withAuthCookieLock(async () => {
        attempts++
      }, controller.signal),
      (error: unknown) => axios.isCancel(error)
    )
    reader.cache.delete(KEY)
    reader.notice(KEY)
    controller.abort()
    await rejected
    assert.equal(attempts, 0)
    assert.equal(reader.cookie.getAuthCookieBlockedState(), null)
  } finally {
    reader.close()
  }
})

test('an already aborted acquisition sends nothing and does not wait or poison state', async () => {
  const reader = realm(new Map([[KEY, marker]]))
  const controller = new AbortController()
  controller.abort()
  let attempts = 0
  try {
    await assert.rejects(
      reader.cookie.withAuthCookieLock(async () => {
        attempts++
      }, controller.signal),
      (error: unknown) => axios.isCancel(error)
    )
    assert.equal(reader.timers.size, 0)
    assert.equal(attempts, 0)
    assert.equal(reader.cookie.getAuthCookieBlockedState(), null)
  } finally {
    reader.close()
  }
})

test('storage failure during handoff rejects without dispatch and cleans up', async () => {
  const reader = realm(new Map([[KEY, marker]]))
  let attempts = 0
  try {
    const rejected = assert.rejects(
      reader.cookie.withAuthCookieLock(async () => {
        attempts++
      }),
      reader.cookie.AuthCookieBlockedError
    )
    reader.failReads()
    reader.notice(KEY)
    await rejected
    assert.equal(
      reader.cookie.getAuthCookieBlockedState(),
      'storage_unavailable'
    )
    assert.equal(attempts, 0)
  } finally {
    reader.close()
  }
})
