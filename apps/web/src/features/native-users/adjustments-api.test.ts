/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

import { readNativeUsersScope } from './access'
import {
  adjustmentDraftSchema,
  adjustUserBalance,
  readAdjustmentReceipt,
} from './adjustments-api'

const adapter = api.defaults.adapter
const previous = useAuthStore.getState().auth
const draft = {
  operation_key: 'synthetic-adjustment-operation-0001',
  delta_units: '-9007199254740993',
  reason: 'Verified correction',
}
const receipt = {
  contract: 'core.admin-user-adjustment.v1',
  operation_key: draft.operation_key,
  user_id: '9007199254740997',
  delta_units: draft.delta_units,
  status: 'posted',
  journal_id: '9007199254740999',
  rejection: null,
}
function login(sid = 'synthetic-adjustment-session') {
  useAuthStore.getState().auth.setBundle({
    access_token: 'synthetic-adjustment-bearer',
    token_type: 'Bearer',
    access_expires_at: 2000000000,
    user: { id: 501, role: 10, username: 'synthetic-admin' },
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
  const scope = readNativeUsersScope()
  assert.ok(scope)
  return scope
}
afterEach(() => {
  api.defaults.adapter = adapter
  useAuthStore.setState({ auth: previous })
})

test('adjustments preserve exact signed integer money, permanent operation key and receipt identity', async () => {
  const scope = login()
  api.defaults.adapter = async (config) => {
    assert.equal(config.skipAuthRefresh, true)
    assert.deepEqual(config.authScope, {
      userId: 501,
      sessionId: scope.sessionId,
    })
    if (config.method === 'post') {
      assert.deepEqual(JSON.parse(String(config.data)), draft)
    } else {
      assert.equal(
        config.url,
        `/core/v1/admin/user-balances/${receipt.user_id}/adjustments/${draft.operation_key}`
      )
    }
    return { config, data: receipt, headers: {}, status: 200, statusText: 'OK' }
  }
  assert.equal(
    (await adjustUserBalance(scope, receipt.user_id, draft)).journal_id,
    receipt.journal_id
  )
  assert.deepEqual(
    await readAdjustmentReceipt(scope, receipt.user_id, draft),
    receipt
  )
})

test('ambiguous amounts, unknown response identities and envelope changes cannot report a financial success', async () => {
  const scope = login()
  for (const delta_units of [
    '0',
    '-0',
    '01',
    '+1',
    '1e4',
    '1.2',
    '-9223372036854775808',
  ]) {
    assert.equal(
      adjustmentDraftSchema.safeParse({ ...draft, delta_units }).success,
      false
    )
  }
  for (const data of [
    { ...receipt, operation_key: 'synthetic-adjustment-operation-9999' },
    { ...receipt, delta_units: '-1' },
    { ...receipt, user_id: '999' },
    { success: true, data: receipt },
  ]) {
    api.defaults.adapter = async (config) => ({
      config,
      data,
      headers: {},
      status: 200,
      statusText: 'OK',
    })
    await assert.rejects(
      adjustUserBalance(scope, receipt.user_id, draft),
      /unknown/
    )
  }
})

test('lost responses never replay automatically and explicit recovery uses the original key', async () => {
  const scope = login()
  const writes: unknown[] = []
  api.defaults.adapter = async (config) => {
    writes.push(JSON.parse(String(config.data)))
    if (writes.length === 1) throw new Error('commit acknowledgement lost')
    return { config, data: receipt, headers: {}, status: 200, statusText: 'OK' }
  }
  await assert.rejects(
    adjustUserBalance(scope, receipt.user_id, draft),
    /unknown/
  )
  assert.equal(writes.length, 1)
  await adjustUserBalance(scope, receipt.user_id, draft)
  assert.deepEqual(writes, [draft, draft])
  login('replacement-adjustment-session')
  await assert.rejects(
    adjustUserBalance(scope, receipt.user_id, draft),
    /session_changed/
  )
  assert.equal(writes.length, 2)
})
