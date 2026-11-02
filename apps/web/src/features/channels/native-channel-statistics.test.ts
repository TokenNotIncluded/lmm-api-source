/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

import { readNativeRoutingScope } from './native-api'
import {
  channelStatisticsSchema,
  getNativeChannelPage,
  isChannelSnapshotChanged,
  type NativeChannelSortInput,
} from './native-channel-statistics'

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

const input: NativeChannelSortInput = {
  sort: 'lifetime',
  order: 'desc',
  unit: '',
  page: 1,
  search: '',
}
const row = {
  id: 1,
  lifetime: {
    complete: true,
    total_usd: '18014398509.481986',
    exact_usd_numerator: '9007199254740993',
    exact_usd_denominator: '500000',
  },
  observation: null,
  response_time: null,
  test_time: null,
  catalog_observation: null,
}
function page() {
  return {
    items: [row],
    total: 30,
    page: 1,
    page_size: 25,
    config_version: 1,
    pricing_version: 1,
    statistics_scope: 'channel_display_v1',
    probe_scope: 'model_catalog',
    inference_tested: false,
    snapshot_digest: 'a'.repeat(64),
    pagination_consistency: 'single_request',
    statistics_snapshot_at: 1,
    balance_units: ['CNY', 'USD'],
    sort_unit: null,
  }
}
test('native sorted pages preserve exact strings and send the original digest', async () => {
  const scope = login()
  let data = page()
  let url = ''
  api.defaults.adapter = async (config) => {
    url = config.url ?? ''
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: { success: true, data },
    }
  }
  const first = await getNativeChannelPage(
    scope,
    { routing: 1, pricing: 1 },
    input
  )
  assert.equal(first.items[0].lifetime.exact_usd_numerator, '9007199254740993')
  data = { ...data, page: 2, pagination_consistency: 'digest_checked' }
  await getNativeChannelPage(
    scope,
    { routing: 1, pricing: 1 },
    { ...input, page: 2, snapshotDigest: first.snapshot_digest }
  )
  const query = new URL(url, 'http://localhost').searchParams
  assert.equal(query.get('p'), '2')
  assert.equal(query.get('snapshot_digest'), 'a'.repeat(64))
})
test('balance sorting requires an explicit unit before dispatch', async () => {
  const scope = login()
  let calls = 0
  api.defaults.adapter = async (config) => {
    calls++
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: { success: true, data: { ...page(), sort_unit: 'USD' } },
    }
  }
  await assert.rejects(
    getNativeChannelPage(
      scope,
      { routing: 1, pricing: 1 },
      { ...input, sort: 'balance' }
    )
  )
  assert.equal(calls, 0)
  await getNativeChannelPage(
    scope,
    { routing: 1, pricing: 1 },
    { ...input, sort: 'balance', unit: 'USD' }
  )
  assert.equal(calls, 1)
})
test('changed configuration or unchecked continuation cannot become a valid page', async () => {
  const scope = login()
  let data = page()
  api.defaults.adapter = async (config) => ({
    config,
    headers: {},
    status: 200,
    statusText: 'OK',
    data: { success: true, data },
  })
  await assert.rejects(
    getNativeChannelPage(scope, { routing: 2, pricing: 1 }, input)
  )
  await assert.rejects(
    getNativeChannelPage(
      scope,
      { routing: 1, pricing: 1 },
      { ...input, snapshotDigest: 'a'.repeat(64) }
    )
  )
  data = {
    ...data,
    snapshot_digest: 'b'.repeat(64),
    pagination_consistency: 'digest_checked',
  }
  await assert.rejects(
    getNativeChannelPage(
      scope,
      { routing: 1, pricing: 1 },
      { ...input, snapshotDigest: 'a'.repeat(64) }
    )
  )
  assert.equal(
    isChannelSnapshotChanged({
      response: { status: 409, data: { code: 'CHANNEL_SNAPSHOT_CHANGED' } },
    }),
    true
  )
  assert.equal(
    isChannelSnapshotChanged({
      response: { status: 409, data: { code: 'OTHER' } },
    }),
    false
  )
})
test('unknown or failed old probes cannot masquerade as zero inference latency', () => {
  assert.equal(
    channelStatisticsSchema.safeParse({ ...row, response_time: 0 }).success,
    false
  )
  assert.equal(
    channelStatisticsSchema.safeParse({
      ...row,
      lifetime: { ...row.lifetime, complete: false },
    }).success,
    false
  )
  assert.equal(
    channelStatisticsSchema.safeParse({
      ...row,
      test_time: 1,
      catalog_observation: {
        scope: 'inference',
        inference_tested: true,
        result: 'success',
        duration_ms: 0,
        observed_at: 1,
      },
    }).success,
    false
  )
  assert.equal(
    channelStatisticsSchema.safeParse({
      ...row,
      test_time: 1,
      catalog_observation: {
        scope: 'model_catalog',
        inference_tested: false,
        result: 'failed',
        duration_ms: 20,
        observed_at: 1,
      },
    }).success,
    true
  )
})
