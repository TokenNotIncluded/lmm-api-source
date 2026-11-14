/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import {
  createQueuedLockManager,
  installSupportedAuthBrowser,
} from './supported-auth-browser'

function deferred() {
  let resolve!: () => void
  const promise = new Promise<void>((done) => {
    resolve = done
  })
  return { promise, resolve }
}

test('same-name requests exclude and queue FIFO while a different name proceeds', async () => {
  const locks = createQueuedLockManager(),
    held = deferred(),
    entered = deferred()
  const order: string[] = []
  const first = locks.request('auth', {}, async () => {
    order.push('first')
    entered.resolve()
    await held.promise
  })
  await entered.promise
  const second = locks.request('auth', {}, () => {
    order.push('second')
  })
  const third = locks.request('auth', {}, () => {
    order.push('third')
  })
  await locks.request('other', {}, () => {
    order.push('other')
  })
  assert.deepEqual(order, ['first', 'other'])
  held.resolve()
  await Promise.all([first, second, third])
  assert.deepEqual(order, ['first', 'other', 'second', 'third'])
})

test('queued/pre-aborted requests never run; abort after grant never releases an active callback', async () => {
  const locks = createQueuedLockManager(),
    held = deferred(),
    entered = deferred()
  const active = new AbortController(),
    queued = new AbortController()
  const first = locks.request('auth', { signal: active.signal }, async () => {
    entered.resolve()
    await held.promise
  })
  await entered.promise
  let dispatched = 0
  const cancelled = locks.request('auth', { signal: queued.signal }, () => {
    dispatched++
  })
  const rejected = assert.rejects(cancelled, { name: 'AbortError' })
  queued.abort()
  await rejected
  await assert.rejects(
    locks.request('auth', { signal: queued.signal }, () => {
      dispatched++
    }),
    { name: 'AbortError' }
  )
  const last = locks.request('auth', {}, () => {
    dispatched++
  })
  active.abort()
  await Promise.resolve()
  await Promise.resolve()
  assert.equal(dispatched, 0)
  held.resolve()
  await Promise.all([first, last])
  assert.equal(dispatched, 1)
})

test('a rejected callback releases exclusion for its queued successor', async () => {
  const locks = createQueuedLockManager(),
    held = deferred(),
    entered = deferred()
  const first = locks.request('auth', {}, async () => {
    entered.resolve()
    await held.promise
    throw new Error('synthetic failure')
  })
  const failed = assert.rejects(first, /synthetic failure/)
  await entered.promise
  const next = locks.request('auth', {}, () => 42)
  held.resolve()
  await failed
  assert.equal(await next, 42)
})

test('explicit capability installation preserves storage denial/markers and restores descriptors', () => {
  const secure = Object.getOwnPropertyDescriptor(globalThis, 'isSecureContext')
  const win = Object.getOwnPropertyDescriptor(globalThis, 'window')
  const storage = {
    getItem: () => 'uncertain:keep',
    setItem: () => {
      throw new Error('denied')
    },
  }
  const browser = { localStorage: storage }
  const restore = installSupportedAuthBrowser({ window: browser })
  assert.equal(globalThis.isSecureContext, true)
  assert.equal(browser.localStorage.getItem(), 'uncertain:keep')
  assert.throws(() => browser.localStorage.setItem(), /denied/)
  assert.equal(typeof navigator.locks.request, 'function')
  restore()
  assert.deepEqual(Object.getOwnPropertyDescriptor(globalThis, 'window'), win)
  assert.deepEqual(
    Object.getOwnPropertyDescriptor(globalThis, 'isSecureContext'),
    secure
  )
  assert.deepEqual(Object.keys(browser), ['localStorage'])
})
