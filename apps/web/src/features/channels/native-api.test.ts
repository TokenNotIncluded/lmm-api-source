/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

import {
  assertNativeIntegers,
  copyNativeChannel,
  nativeRequest,
  parseNativeDocument,
  readNativeRoutingScope,
} from './native-api'

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

test('native routing rejects rounded identifiers and monetary integers before dispatch', async () => {
  const scope = login()
  let calls = 0
  api.defaults.adapter = async () => {
    calls++
    throw new Error('must not dispatch')
  }
  for (const value of [9_007_199_254_740_992, 1.5, Number.NaN, Infinity]) {
    assert.throws(
      () => assertNativeIntegers({ rules: [{ id: value }] }),
      /safely/
    )
    await assert.rejects(
      nativeRequest(scope, 'PUT', '/core/v1/admin/routing/prices', {
        expected_version: { routing: 1, pricing: 1 },
        rates: { input: value },
      }),
      /safely/
    )
  }
  assert.throws(() => parseNativeDocument('{"id":9007199254740993}'), /safely/)
  assert.throws(() => parseNativeDocument('[]'), /object/)
  assert.equal(calls, 0)
})

test('native publications keep exact version CAS, never refresh/replay and reject malformed success receipts', async () => {
  const scope = login()
  const body = { expected_version: { routing: 4, pricing: 3 }, enabled: false }
  let calls = 0
  api.defaults.adapter = async (config) => {
    calls++
    assert.equal(config.skipAuthRefresh, true)
    assert.deepEqual(config.authScope, {
      userId: 501,
      sessionId: scope.sessionId,
    })
    assert.deepEqual(JSON.parse(String(config.data)), body)
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: { success: true, data: { version: { routing: 5, pricing: 3 } } },
    }
  }
  assert.deepEqual(
    await nativeRequest(scope, 'PUT', '/api/channel/1/status', body),
    { version: { routing: 5, pricing: 3 } }
  )
  assert.equal(calls, 1)
  for (const data of [
    { success: false, message: 'Denied', data: {} },
    { success: true, data: {} },
    { success: true, data: { version: { routing: 4, pricing: 3 } } },
  ]) {
    api.defaults.adapter = async (config) => ({
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data,
    })
    await assert.rejects(
      nativeRequest(scope, 'PUT', '/api/channel/1/status', body)
    )
  }
  calls = 0
  api.defaults.adapter = async () => {
    calls++
    throw new Error('commit acknowledgement lost')
  }
  await assert.rejects(
    nativeRequest(scope, 'PUT', '/api/channel/1/status', body),
    /acknowledgement lost/
  )
  assert.equal(calls, 1)
})

test('a changed administrator session cannot reuse a previous editor or accept a late private response', async () => {
  const scope = login()
  let calls = 0
  api.defaults.adapter = async (config) => {
    calls++
    login('replacement-admin-session')
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: { success: true, data: { private: true } },
    }
  }
  await assert.rejects(
    nativeRequest(scope, 'GET', '/core/v1/admin/routing/configuration')
  )
  assert.equal(calls, 1)
  await assert.rejects(
    nativeRequest(scope, 'PUT', '/api/channel/1/status', {
      expected_version: { routing: 1, pricing: 1 },
      enabled: true,
    })
  )
  assert.equal(calls, 1)
})

test('native copy sends exact version, suffix and reset mode and validates the new identity and publication', async () => {
  const scope = login()
  const expected = { routing: 4, pricing: 3 }
  let calls = 0
  let receipt: unknown = { id: 9, version: { routing: 5, pricing: 3 } }
  let suffix = '_copy'
  let resetBalance = true
  api.defaults.adapter = async (config) => {
    calls++
    assert.equal(config.method, 'post')
    assert.equal(config.url, '/core/v1/admin/routing/channels/1/copy')
    assert.equal(config.skipAuthRefresh, true)
    assert.deepEqual(JSON.parse(String(config.data)), {
      expected_version: expected,
      suffix,
      ...(resetBalance ? {} : { reset_balance: false }),
    })
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: { success: true, data: receipt },
    }
  }
  assert.deepEqual(await copyNativeChannel(scope, 1, expected, suffix), receipt)
  suffix = ''
  await copyNativeChannel(scope, 1, expected, suffix)
  resetBalance = false
  await copyNativeChannel(scope, 1, expected, suffix, resetBalance)
  for (const invalid of [
    {},
    { id: 0, version: { routing: 5, pricing: 3 } },
    { id: 1, version: { routing: 5, pricing: 3 } },
    { id: 1.5, version: { routing: 5, pricing: 3 } },
    { id: 9007199254740992, version: { routing: 5, pricing: 3 } },
    { id: 9, version: { routing: 4, pricing: 3 } },
    { id: 9, version: { routing: 5, pricing: 4 } },
  ]) {
    receipt = invalid
    await assert.rejects(
      copyNativeChannel(scope, 1, expected, suffix, resetBalance)
    )
  }
  assert.equal(calls, 10)
  const before = calls
  await assert.rejects(copyNativeChannel(scope, 0, expected, suffix))
  assert.equal(calls, before)
})

test('native copy does not replay a lost acknowledgement or accept a response after session drift', async () => {
  const scope = login()
  let calls = 0
  api.defaults.adapter = async () => {
    calls++
    throw new Error('copy acknowledgement lost')
  }
  await assert.rejects(
    copyNativeChannel(scope, 1, { routing: 1, pricing: 1 }, '_copy'),
    /acknowledgement lost/
  )
  assert.equal(calls, 1)
  api.defaults.adapter = async (config) => {
    calls++
    login('replacement-copy-session')
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: {
        success: true,
        data: { id: 9, version: { routing: 2, pricing: 1 } },
      },
    }
  }
  await assert.rejects(
    copyNativeChannel(scope, 1, { routing: 1, pricing: 1 }, '_copy')
  )
  assert.equal(calls, 2)
})

test('native channel usage keeps exact strings, server-default windows and session-scoped cancellation', async () => {
  const { getNativeChannelUsage } = await import('./native-api')
  const scope = login()
  const controller = new AbortController()
  const usage = {
    scope: 'native_channel_window',
    start_timestamp: 1_700_000_000,
    end_timestamp: 1_700_000_000 + 31 * 86400,
    gross_settled_credits: '9007199254740993',
    refunded_credits: '2',
    net_used_credits: '9007199254740991',
    settled_requests: '9007199254740993',
    pending_potential_requests: '4',
    unknown_provenance_requests: '5',
    credits_per_usd: '500000',
    historical_coverage: false,
  }
  let payload: unknown = usage
  let calls = 0
  api.defaults.adapter = async (config) => {
    calls++
    assert.equal(config.method, 'get')
    assert.equal(config.url, '/core/v1/admin/routing/channels/1/usage')
    assert.equal(config.signal, controller.signal)
    assert.equal(config.skipAuthRefresh, true)
    assert.deepEqual(config.authScope, {
      userId: 501,
      sessionId: scope.sessionId,
    })
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: { success: true, data: payload },
    }
  }
  assert.deepEqual(
    await getNativeChannelUsage(scope, 1, controller.signal),
    usage
  )
  for (const invalid of [
    { ...usage, gross_settled_credits: 9007199254740992 },
    { ...usage, settled_requests: undefined },
    { ...usage, pending_potential_requests: null },
    { ...usage, net_used_credits: '1.5' },
    { ...usage, historical_coverage: true },
    { ...usage, scope: 'lifetime' },
    { ...usage, credits_per_usd: '3650000' },
    { ...usage, end_timestamp: usage.end_timestamp + 1 },
  ]) {
    payload = invalid
    await assert.rejects(getNativeChannelUsage(scope, 1, controller.signal))
  }
  const before = calls
  const stale = { ...scope, sessionId: 'old-session' }
  await assert.rejects(
    getNativeChannelUsage(stale, 1, controller.signal),
    /session changed/
  )
  assert.equal(calls, before)
})

test('provider observations preserve exact decimals and units, while probe writes require valid same-origin paths and CAS', async () => {
  const { getNativeChannelBalance, saveNativeBalanceProbe } =
    await import('./native-api')
  const scope = login()
  const version = { routing: 4, pricing: 3 }
  const observation = {
    balance: '9007199254740993.123456789',
    unit_label: 'provider points',
    balance_updated_time: null,
    kind: 'copied',
    recorded_at: 1_700_000_000,
    source_channel_id: 2,
  }
  let calls = 0
  let expectedProbe: unknown = {
    path: '/account/balance',
    json_pointer: '/a~1b/~0balance',
    unit_label: null,
  }
  api.defaults.adapter = async (config) => {
    calls++
    assert.equal(config.skipAuthRefresh, true)
    if (config.method === 'get') {
      assert.equal(config.url, '/core/v1/admin/routing/channels/1/balance')
      return {
        config,
        headers: {},
        status: 200,
        statusText: 'OK',
        data: { success: true, data: observation },
      }
    }
    assert.equal(config.url, '/core/v1/admin/routing/channels/1/balance-config')
    assert.deepEqual(JSON.parse(String(config.data)), {
      expected_version: version,
      probe: expectedProbe,
    })
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: { success: true, data: { version: { routing: 5, pricing: 3 } } },
    }
  }
  assert.deepEqual(await getNativeChannelBalance(scope, 1), observation)
  await saveNativeBalanceProbe(scope, 1, version, {
    path: '/account/balance',
    json_pointer: '/a~1b/~0balance',
    unit_label: null,
  })
  expectedProbe = null
  await saveNativeBalanceProbe(scope, 1, version, null)
  const before = calls
  for (const path of [
    'https://elsewhere.test/balance',
    '//elsewhere.test/balance',
    '/balance?token=secret',
    '/balance#fragment',
    '/\\elsewhere.test',
    '/bad path',
  ]) {
    await assert.rejects(
      saveNativeBalanceProbe(scope, 1, version, {
        path,
        json_pointer: '/balance',
        unit_label: null,
      })
    )
  }
  for (const json_pointer of ['', 'balance', '/bad~2escape', '/dangling~']) {
    await assert.rejects(
      saveNativeBalanceProbe(scope, 1, version, {
        path: '/balance',
        json_pointer,
        unit_label: null,
      })
    )
  }
  assert.equal(calls, before)
})

test('copied display counters keep nullable observations and unknown history without imposing the window contract on native totals', async () => {
  const { getNativeChannelCounters } = await import('./native-api')
  const scope = login()
  const counters = {
    balance: {
      balance: null,
      unit_label: null,
      balance_updated_time: null,
      kind: 'unknown',
      recorded_at: null,
      source_channel_id: null,
    },
    native: {
      scope: 'native_channel_totals',
      start_timestamp: 0,
      end_timestamp: 1_700_000_000,
      gross_settled_credits: '9007199254740993',
      refunded_credits: '1',
      net_used_credits: '9007199254740992',
      settled_requests: '1',
      pending_potential_requests: '0',
      unknown_provenance_requests: '5',
      credits_per_usd: '500000',
      historical_coverage: false,
    },
    inherited: {
      gross_settled_credits: '5',
      refunded_credits: '1',
      net_used_credits: '4',
    },
    combined: {
      gross_settled_credits: '9007199254740998',
      refunded_credits: '2',
      net_used_credits: '9007199254740996',
    },
    historical_coverage: false,
    complete: false,
    used_quota: null,
    credits_per_usd: '500000',
  }
  let payload: unknown = counters
  api.defaults.adapter = async (config) => {
    assert.equal(config.url, '/core/v1/admin/routing/channels/1/counters')
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: { success: true, data: payload },
    }
  }
  const read = await getNativeChannelCounters(scope, 1)
  assert.equal(read.used_quota, null)
  assert.equal(read.balance.recorded_at, null)
  assert.equal(read.native.net_used_credits, '9007199254740992')
  assert.equal(read.combined.net_used_credits, '9007199254740996')
  for (const invalid of [
    { ...counters, used_quota: '0' },
    { ...counters, complete: true },
    { ...counters, combined: { ...counters.combined, net_used_credits: '0' } },
    {
      ...counters,
      native: { ...counters.native, scope: 'native_channel_window' },
    },
  ]) {
    payload = invalid
    await assert.rejects(getNativeChannelCounters(scope, 1))
  }
})
