/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import { readNativeUsersScope } from '@/features/native-users/access'
import { useAuthStore } from '@/stores/auth-store'

import {
  originalGroupDescriptions,
  parsePricingSourceSnapshot,
  readPricingSourceSnapshot,
} from './pricing-source-snapshot-api'

const previousAuth = useAuthStore.getState().auth
afterEach(() => useAuthStore.setState({ auth: previousAuth }))

function fixture() {
  return {
    contract: 'core.legacy-pricing-snapshot.v1',
    purpose: 'reference_only',
    source_sha256: 'a'.repeat(64),
    observed_at: 1794852236,
    options: [
      {
        key: 'ModelRatio',
        value: '{ "synthetic-image": 1.2300e-7, "precise": 9007199254740993 }',
      },
      { key: 'GroupRatio', value: '{"default":0.7500}' },
    ],
    enabled_channels: [
      {
        id: 1,
        name: 'Source channel',
        models: 'synthetic-image',
        groups: 'default',
        model_mapping: '',
      },
    ],
    unsupported: [
      {
        model: 'synthetic-image',
        group: 'default',
        reason: 'media_contract_required',
        source_channel_ids: [1],
      },
    ],
  }
}
function session(role = 100, sid = 'synthetic-snapshot-session') {
  useAuthStore.getState().auth.setBundle({
    access_token: 'synthetic-snapshot-token',
    token_type: 'Bearer',
    access_expires_at: 2_000_000_000,
    user: { id: 501, role, username: 'synthetic-admin' },
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
}
test('snapshot schema preserves raw numeric text and rejects credentials, extra fields and unbound references', () => {
  const value = fixture()
  const snapshot = parsePricingSourceSnapshot({ success: true, data: value })
  assert.ok(snapshot)
  assert.deepEqual(snapshot, value)
  assert.match(snapshot.options[0].value, /1\.2300e-7/)
  assert.match(snapshot.options[0].value, /9007199254740993/)
  assert.equal(parsePricingSourceSnapshot({ success: true, data: null }), null)
  for (const bad of [
    { ...fixture(), purpose: 'active_billing' },
    { ...fixture(), secret: 'untrusted-secret' },
    {
      ...fixture(),
      options: [{ key: 'StripeSecretKey', value: 'untrusted-secret' }],
    },
    {
      ...fixture(),
      enabled_channels: [
        { ...fixture().enabled_channels[0], key: 'untrusted-secret' },
      ],
    },
    {
      ...fixture(),
      unsupported: [{ ...fixture().unsupported[0], source_channel_ids: [99] }],
    },
    {
      ...fixture(),
      options: [{ key: 'ModelRatio', value: '1'.repeat(512 * 1024 + 1) }],
    },
  ]) {
    assert.throws(() =>
      parsePricingSourceSnapshot({ success: true, data: bad })
    )
  }
})

test('group descriptions select the legacy usable-groups option and ignore non-text entries', () => {
  const value = fixture()
  value.options.push({
    key: 'UserUsableGroups',
    value: '{"supported":"Displayed description","ignored":0,"empty":""}',
  })
  const snapshot = parsePricingSourceSnapshot({ success: true, data: value })
  assert.deepEqual(
    [...originalGroupDescriptions(snapshot)],
    [['supported', 'Displayed description']]
  )
  assert.equal(originalGroupDescriptions(null).size, 0)
})

test('only administrators with the current session can read group description metadata', async () => {
  session(1)
  assert.equal(readNativeUsersScope(), undefined)
  session()
  const scope = readNativeUsersScope()
  assert.ok(scope)
  session(100, 'replacement-session')
  await assert.rejects(readPricingSourceSnapshot(scope), /Session changed/)
})
