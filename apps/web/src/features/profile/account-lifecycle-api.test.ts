/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, beforeEach, test } from 'node:test'

import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

import {
  AccountClosureFailure,
  closeAccount,
  closureScope,
  prepareClosure,
  readAccountClosure,
  readClosureReceipt,
  readPendingClosure,
  type AccountClosurePreview,
} from './account-lifecycle-api'

const adapter = api.defaults.adapter
const previousAuth = useAuthStore.getState().auth
const previousWindow = Object.getOwnPropertyDescriptor(globalThis, 'window')
const storage = new Map<string, string>()
let refuseStorage = false
Object.defineProperty(globalThis, 'window', {
  configurable: true,
  value: {
    sessionStorage: {
      getItem: (key: string) => storage.get(key) ?? null,
      setItem: (key: string, value: string) => {
        if (refuseStorage) throw new Error('Synthetic storage denial')
        storage.set(key, value)
      },
      removeItem: (key: string) => storage.delete(key),
    },
  },
})
beforeEach(() => {
  storage.clear()
  refuseStorage = false
})
afterEach(() => {
  api.defaults.adapter = adapter
  useAuthStore.setState({ auth: previousAuth })
})
after(() => {
  if (previousWindow) {
    Object.defineProperty(globalThis, 'window', previousWindow)
  } else Reflect.deleteProperty(globalThis, 'window')
})
function login(sid = 'synthetic-closure-family') {
  useAuthStore.getState().auth.setBundle({
    access_token: 'synthetic-closure-access',
    token_type: 'Bearer',
    access_expires_at: 2000000000,
    user: { id: 501, username: 'synthetic-user', role: 1 },
    session: {
      sid,
      current: true,
      login_method: 'password',
      ip: '',
      user_agent: '',
      created_at: 1,
      last_active_at: 1,
      expires_at: 2000000000,
    },
  })
  return closureScope()
}
const preview: AccountClosurePreview = {
  contract: 'core.account-closure.v1',
  expected_auth_version: '9007199254740993',
  username: 'synthetic-user',
  allowed: true,
  blockers: [],
}

test('Closure stores the original operation before dispatch, preserves an unknown result, and reads its terminal receipt', async () => {
  const scope = login()
  const pending = prepareClosure(scope)
  let calls = 0
  api.defaults.adapter = async (config) => {
    calls++
    assert.equal(
      readPendingClosure(scope.userId)?.operation_key,
      pending.operation_key
    )
    assert.equal(config.skipAuthRefresh, true)
    assert.deepEqual(JSON.parse(String(config.data)), {
      expected_auth_version: preview.expected_auth_version,
      confirmation: preview.username,
      operation_key: pending.operation_key,
    })
    throw Object.assign(new Error('Synthetic lost response'), {
      response: { status: 503 },
    })
  }
  await assert.rejects(
    closeAccount(scope, pending, preview, preview.username),
    (error: unknown) =>
      error instanceof AccountClosureFailure && error.kind === 'unknown'
  )
  assert.equal(calls, 1)
  assert.deepEqual(readPendingClosure(scope.userId), pending)
  assert.equal(useAuthStore.getState().auth.session?.sid, scope.sessionId)
  api.defaults.adapter = async (config) => {
    assert.equal(
      config.url,
      `/api/user/self/closure/receipts/${pending.operation_key}`
    )
    assert.equal(config.skipAuthRefresh, true)
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: {
        success: true,
        data: {
          contract: 'core.account-closure.v1',
          operation_key: pending.operation_key,
          closed: true,
        },
      },
    }
  }
  assert.equal((await readClosureReceipt(pending)).closed, true)
})

test('Closure refuses storage failure, unconfirmed usernames, blocked finances and changed sessions before dispatch', async () => {
  const scope = login()
  let calls = 0
  api.defaults.adapter = async () => {
    calls++
    throw new Error('Unexpected dispatch')
  }
  refuseStorage = true
  assert.throws(() => prepareClosure(scope))
  refuseStorage = false
  const pending = prepareClosure(scope)
  await assert.rejects(closeAccount(scope, pending, preview, 'someone else'))
  await assert.rejects(
    closeAccount(
      scope,
      pending,
      { ...preview, allowed: false, blockers: ['unsettled_balance'] },
      preview.username
    )
  )
  login('replacement-family')
  await assert.rejects(closeAccount(scope, pending, preview, preview.username))
  assert.equal(calls, 0)
})

test('Closure eligibility rejects contradictory or imprecise preflight responses', async () => {
  const scope = login()
  for (const data of [
    { ...preview, expected_auth_version: 9007199254740992 },
    { ...preview, blockers: ['active_subscription'] },
  ]) {
    api.defaults.adapter = async (config) => ({
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: { success: true, data },
    })
    await assert.rejects(readAccountClosure(scope))
  }
})
