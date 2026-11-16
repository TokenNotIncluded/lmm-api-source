/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
// @ts-expect-error Bun's test module is only available in the test runtime.
import { afterEach, expect, test } from 'bun:test'

import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

import { readNativeUser } from './api'
import { nativeUserStatisticsFixture } from './detail-test-fixtures'
import { nativeUserFixture, nativeUsageFixture } from './test-fixtures'

const originalAdapter = api.defaults.adapter
const previousAuth = useAuthStore.getState().auth
const scope = {
  userId: 901,
  sessionId: 'synthetic-user-detail',
  role: 10 as const,
}

function login(role = 10, sid = scope.sessionId) {
  useAuthStore.getState().auth.setBundle({
    access_token: 'synthetic-user-detail-credential',
    token_type: 'Bearer',
    access_expires_at: 2000000000,
    user: { id: 901, role, username: 'synthetic-admin' },
    session: {
      sid,
      current: true,
      login_method: 'password',
      ip: '127.0.0.1',
      user_agent: 'synthetic',
      created_at: 1,
      last_active_at: 1,
      expires_at: 2000000000,
    },
  })
}

afterEach(() => {
  api.defaults.adapter = originalAdapter
  useAuthStore.setState({ auth: previousAuth })
})

test('user detail reads one exact ID with scoped native options and exact credit units', async () => {
  login()
  const user = nativeUserFixture()
  const seen: string[] = []
  api.defaults.adapter = async (config) => {
    seen.push(`${config.method} ${config.url}`)
    expect(config.authScope).toEqual({
      userId: 901,
      sessionId: scope.sessionId,
    })
    expect(config.skipAuthRefresh).toBe(true)
    expect(config.disableDuplicate).toBe(true)
    expect(config.timeout).toBe(10000)
    return {
      config,
      data: { version: 1, user, usage: nativeUsageFixture() },
      headers: {},
      status: 200,
      statusText: 'OK',
    }
  }
  expect(await readNativeUser(scope, user.id)).toEqual({
    version: 1,
    user,
    usage: nativeUsageFixture(),
  })
  expect(seen).toEqual(['get /core/v1/admin/users/9007199254740993'])
})

test('a list, old envelope, mismatched user or incomplete wallet cannot become user detail', async () => {
  login()
  const user = nativeUserFixture()
  for (const payload of [
    { success: true, data: user },
    { version: 1, items: [user] },
    {
      version: 1,
      user,
      usage: { ...nativeUsageFixture(), net_consumption_units: '999' },
    },
    { version: 1, usage: nativeUsageFixture(), user: { ...user, id: '12' } },
    {
      version: 1,
      usage: nativeUsageFixture(),
      user: { ...user, wallet: { status: 'available' } },
    },
  ]) {
    api.defaults.adapter = async (config) => ({
      config,
      data: payload,
      headers: {},
      status: 200,
      statusText: 'OK',
    })
    await expect(readNativeUser(scope, user.id)).rejects.toMatchObject({
      kind: 'invalid_response',
    })
  }
})

test('invalid IDs and stale sessions cannot issue or retain native detail reads', async () => {
  login()
  let calls = 0
  api.defaults.adapter = async (config) => {
    calls++
    login(1)
    return {
      config,
      data: {
        version: 1,
        usage: nativeUsageFixture(),
        user: nativeUserFixture(),
      },
      headers: {},
      status: 200,
      statusText: 'OK',
    }
  }
  for (const id of ['0', '-1', '1/../2', '01', '9223372036854775808']) {
    await expect(readNativeUser(scope, id)).rejects.toMatchObject({
      kind: 'invalid_query',
    })
  }
  expect(calls).toBe(0)
  await expect(
    readNativeUser(scope, nativeUserFixture().id)
  ).rejects.toMatchObject({ kind: 'session_changed' })
  expect(calls).toBe(1)
})

test('missing users and denied reads stay failures without legacy fallback or retries', async () => {
  login()
  for (const [status, kind] of [
    [403, 'forbidden'],
    [404, 'unavailable'],
    [503, 'unavailable'],
  ]) {
    const seen: string[] = []
    api.defaults.adapter = async (config) => {
      seen.push(config.url ?? '')
      throw Object.assign(new Error('synthetic private diagnostic'), {
        response: { status },
      })
    }
    await expect(readNativeUser(scope, '10')).rejects.toMatchObject({ kind })
    expect(seen).toEqual(['/core/v1/admin/users/10'])
  }
})

test('statistics retain exact units, signed history and nullable prices for the same subject', async () => {
  login()
  const user = nativeUserFixture()
  const statistics = nativeUserStatisticsFixture(user)
  if (statistics.usage.status !== 'available') {
    throw new Error('Expected usage fixture')
  }
  statistics.usage.requests = '340282366920938463463374607431768211455'
  statistics.usage.credits = null
  statistics.usage.pricing_available = false
  statistics.referrals.affiliate_balance_units = '-9223372036854775808'
  if (statistics.historical_counters.status !== 'available') {
    throw new Error('Expected historical counter fixture')
  }
  statistics.historical_counters.used_quota = '-9223372036854775808'
  statistics.historical_counters.request_count = '9007199254740993'
  statistics.historical_counters.affiliate_rewards = '9223372036854775807'
  api.defaults.adapter = async (config) => ({
    config,
    data: { version: 1, user, usage: nativeUsageFixture(), statistics },
    headers: {},
    status: 200,
    statusText: 'OK',
  })
  expect(await readNativeUser(scope, user.id)).toEqual({
    version: 1,
    user,
    usage: nativeUsageFixture(),
    statistics,
  })
})

test('foreign statistics, mixed units, rounded numbers and invented prices fail closed', async () => {
  login()
  const user = nativeUserFixture()
  const base = nativeUserStatisticsFixture(user)
  for (const statistics of [
    { ...base, user_id: '12' },
    { ...base, personal_account_id: '12' },
    { ...base, usage: { ...base.usage, tokens: 12 } },
    {
      ...base,
      usage: {
        ...base.usage,
        tokens: '340282366920938463463374607431768211456',
      },
    },
    {
      ...base,
      usage: { ...base.usage, credits: null, pricing_available: true },
    },
    { ...base, usage: { ...base.usage, unit: 'legacy_raw_units' } },
    {
      ...base,
      historical_counters: { ...base.historical_counters, unit: 'core_credit' },
    },
    { ...base, referrals: { ...base.referrals, invited_users: '-1' } },
    {
      ...base,
      historical_counters: {
        ...base.historical_counters,
        used_quota: '-9223372036854775809',
      },
    },
  ]) {
    api.defaults.adapter = async (config) => ({
      config,
      data: { version: 1, user, usage: nativeUsageFixture(), statistics },
      headers: {},
      status: 200,
      statusText: 'OK',
    })
    await expect(readNativeUser(scope, user.id)).rejects.toMatchObject({
      kind: 'invalid_response',
    })
  }
})
