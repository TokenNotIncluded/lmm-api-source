/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, test } from 'node:test'

import axios from 'axios'

import { login } from '@/features/auth/api'
import { useAuthStore } from '@/stores/auth-store'

import { api } from './api'
import {
  AuthCookieBlockedError,
  getAuthCookieBlockedState,
  isAuthCookieIdle,
  withAuthCookieLock,
} from './auth-cookie'

const descriptors = new Map(
  ['window', 'navigator', 'isSecureContext'].map((key) => [
    key,
    Object.getOwnPropertyDescriptor(globalThis, key),
  ])
)
const storage = new Map([['existing-session-sentinel', 'preserve']])
const before = new Map(storage)
Object.defineProperty(globalThis, 'window', {
  configurable: true,
  value: {
    localStorage: {
      getItem: (key: string) => storage.get(key) ?? null,
      setItem: (key: string, value: string) => storage.set(key, value),
      removeItem: (key: string) => storage.delete(key),
    },
  },
})
Object.defineProperty(globalThis, 'navigator', {
  configurable: true,
  value: {},
})
Object.defineProperty(globalThis, 'isSecureContext', {
  configurable: true,
  value: true,
})
after(() => {
  for (const [key, descriptor] of descriptors) {
    if (descriptor) Object.defineProperty(globalThis, key, descriptor)
    else Reflect.deleteProperty(globalThis, key)
  }
})

test('missing cross-tab coordination rejects before any Cookie writer or state commit', async () => {
  let entered = 0
  let dispatched = 0
  await assert.rejects(
    withAuthCookieLock(async (transaction) => {
      entered++
      return transaction.request(async () => {
        dispatched++
      })
    }),
    AuthCookieBlockedError
  )
  assert.equal(entered, 0)
  assert.equal(dispatched, 0)
  assert.equal(getAuthCookieBlockedState(), 'coordination_unavailable')
  assert.deepEqual(storage, before)
  assert.equal(isAuthCookieIdle(), true)
})

test('an already-cancelled unsupported writer remains cancelled without dispatch', async () => {
  const controller = new AbortController()
  controller.abort()
  let dispatched = 0
  await assert.rejects(
    withAuthCookieLock(async () => {
      dispatched++
    }, controller.signal),
    (error: unknown) => axios.isCancel(error)
  )
  assert.equal(dispatched, 0)
  assert.deepEqual(storage, before)
  assert.equal(isAuthCookieIdle(), true)
})

test('an already-blocked password login preserves storage and auth state without dispatch', async () => {
  assert.equal(getAuthCookieBlockedState(), 'coordination_unavailable')
  const authBefore = useAuthStore.getState().auth
  let commits = 0
  const unsubscribe = useAuthStore.subscribe(() => {
    commits++
  })
  const original = api.defaults.adapter
  let requests = 0
  api.defaults.adapter = async () => {
    requests++
    throw new Error('Unsupported browser must not send a Cookie writer')
  }
  try {
    await assert.rejects(
      login({ username: 'synthetic', password: 'synthetic' }),
      AuthCookieBlockedError
    )
    assert.equal(requests, 0)
    assert.equal(commits, 0)
    assert.equal(useAuthStore.getState().auth, authBefore)
    assert.deepEqual(storage, before)
  } finally {
    unsubscribe()
    api.defaults.adapter = original
  }
})
