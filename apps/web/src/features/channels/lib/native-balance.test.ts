/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import { AxiosError } from 'axios'

import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

import { getNativeChannelBalance, readNativeRoutingScope } from '../native-api'
import {
  refreshNativeChannelBalances,
  type NativeBalanceRefreshResult,
} from './native-balance'

const adapter = api.defaults.adapter
const previous = useAuthStore.getState().auth
function login(sid = 'synthetic-channel-administration') {
  useAuthStore.getState().auth.setBundle({
    access_token: 'synthetic-channel-session-token',
    token_type: 'Bearer',
    access_expires_at: 2_000_000_000,
    user: { id: 501, role: 10, username: 'synthetic-admin' },
    session: {
      sid,
      current: true,
      login_method: 'password',
      ip: '127.0.0.1',
      user_agent: 'synthetic',
      created_at: 1,
      last_active_at: 1,
      expires_at: 2_000_000_000,
    },
  })
  const scope = readNativeRoutingScope()
  assert.ok(scope)
  return scope
}
afterEach(() => {
  api.defaults.adapter = adapter
  useAuthStore.setState({ auth: previous })
})

function observation(balance: string) {
  return {
    balance,
    unit_label: 'USD',
    balance_updated_time: 1_700_000_000,
    kind: 'provider',
    recorded_at: 1_700_000_000,
    source_channel_id: null,
  }
}

test('single and bounded batch refresh use identical requests and observations with isolated errors', async () => {
  const scope = login()
  const calls: number[] = []
  let block = false
  let release: (() => void) | undefined
  api.defaults.adapter = async (config) => {
    assert.equal(config.method, 'get')
    assert.match(
      config.url ?? '',
      /^\/core\/v1\/admin\/routing\/channels\/[1-3]\/balance$/
    )
    const id = Number(config.url?.split('/').at(-2))
    calls.push(id)
    if (block && id === 1) {
      await new Promise<void>((resolve) => {
        release = resolve
      })
    }
    if (id === 2) {
      throw new AxiosError(
        'reflected-private-detail',
        'ERR_BAD_RESPONSE',
        config,
        undefined,
        {
          config,
          status: 502,
          statusText: 'Bad Gateway',
          headers: {},
          data: { error: 'reflected-private-detail' },
        }
      )
    }
    return {
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: {
        success: true,
        data: observation(id === 1 ? '42.5' : '986.17194888'),
      },
    }
  }
  const single = await getNativeChannelBalance(scope, 1)
  calls.length = 0
  block = true
  const results: NativeBalanceRefreshResult[] = []
  const pending = refreshNativeChannelBalances(
    scope,
    [1, 1, 2, 3],
    new AbortController().signal,
    (result) => results.push(result)
  )
  await new Promise((resolve) => setTimeout(resolve, 0))
  assert.deepEqual(calls, [1])
  assert.deepEqual(results, [])
  assert.ok(release)
  release()
  await pending
  assert.deepEqual(calls, [1, 2, 3])
  assert.deepEqual(results, [
    { channelId: 1, status: 'ready', observation: single },
    { channelId: 2, status: 'error', message: 'channelBalance.error' },
    { channelId: 3, status: 'ready', observation: observation('986.17194888') },
  ])
  assert.equal(
    JSON.stringify(results).includes('reflected-private-detail'),
    false
  )
})

test('closing a batch aborts pending work and never starts remaining channels', async () => {
  const scope = login()
  const controller = new AbortController()
  let release: (() => void) | undefined
  let calls = 0
  api.defaults.adapter = async (config) => {
    calls++
    await new Promise<void>((resolve) => {
      release = resolve
    })
    return {
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: { success: true, data: observation('42.5') },
    }
  }
  const results: NativeBalanceRefreshResult[] = []
  const pending = refreshNativeChannelBalances(
    scope,
    [1, 2, 3],
    controller.signal,
    (result) => results.push(result)
  )
  await new Promise((resolve) => setTimeout(resolve, 0))
  assert.ok(release)
  controller.abort()
  release()
  await pending
  assert.equal(calls, 1)
  assert.deepEqual(results, [])
})

test('a disabled probe stays local to its channel while denied access stops the batch', async () => {
  const scope = login()
  const calls: number[] = []
  api.defaults.adapter = async (config) => {
    const id = Number(config.url?.split('/').at(-2))
    calls.push(id)
    throw new AxiosError(
      'private detail',
      'ERR_BAD_RESPONSE',
      config,
      undefined,
      {
        config,
        status: id === 1 ? 501 : 403,
        statusText: 'Unavailable',
        headers: {},
        data: {},
      }
    )
  }
  const results: NativeBalanceRefreshResult[] = []
  await refreshNativeChannelBalances(
    scope,
    [1, 2, 3],
    new AbortController().signal,
    (result) => results.push(result)
  )
  assert.deepEqual(calls, [1, 2])
  assert.deepEqual(results, [
    { channelId: 1, status: 'error', message: 'channelBalance.disabled' },
    { channelId: 2, status: 'error', message: 'channelBalance.denied' },
  ])
})

test('the batch accepts at most one page of valid channel identities before any I/O', async () => {
  const scope = login()
  let calls = 0
  api.defaults.adapter = async () => {
    calls++
    throw new Error('Unexpected I/O')
  }
  for (const ids of [
    [0],
    [-1],
    [1.5],
    Array.from({ length: 26 }, (_, index) => index + 1),
  ]) {
    await assert.rejects(
      refreshNativeChannelBalances(
        scope,
        ids,
        new AbortController().signal,
        () => {}
      ),
      /one page/
    )
  }
  assert.equal(calls, 0)
})
