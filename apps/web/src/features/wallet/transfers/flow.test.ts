/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { afterEach, beforeEach, test } from 'node:test'

import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

import {
  cancelTransfer,
  getTransferBalance,
  claimTransfer,
  inspectTransfer,
  createTransfer,
  queryTransfer,
  queryTransferByID,
} from './api'
import {
  canShareTransfer,
  hasRecipientDetails,
  pendingSchema,
  reconcileTransfer,
  trustedConfirmationURL,
  transferSchema,
  type PendingTransfer,
} from './flow'

const row = transferSchema.parse({
  id: 17,
  quota: 125,
  status: 'awaiting_confirmation',
  created_at: 1,
  request_key: 'immutable-wallet-key',
})
const pending: PendingTransfer = {
  action: 'create',
  requestKey: 'immutable-wallet-key',
  quota: 125,
  step: { kind: 'unknown' },
}
const scope = { userId: 52, sessionId: 'synthetic-transfer-session' }
const originalAuth = useAuthStore.getState().auth
beforeEach(() => {
  useAuthStore.getState().auth.setBundle({
    user: { id: scope.userId, role: 1, username: 'synthetic' },
    access_token: 'synthetic-access-token',
    token_type: 'Bearer',
    access_expires_at: 9999999999,
    session: {
      sid: scope.sessionId,
      current: true,
      login_method: 'password',
      ip: null,
      user_agent: null,
      created_at: 1,
      last_active_at: 1,
      expires_at: 9999999999,
    },
  })
})
const originalPost = api.post,
  originalGet = api.get
afterEach(() => {
  api.post = originalPost
  api.get = originalGet
  useAuthStore.setState({ auth: originalAuth })
})
test('only exact configured Core confirmation URL may open', () => {
  const url = 'https://core.example/core/v1/wallet/confirm/create/17'
  assert.equal(
    trustedConfirmationURL(
      url,
      'create',
      17,
      'https://web.example',
      'https://core.example'
    ),
    url
  )
  for (const raw of [
    `${url}?evil`,
    `${url}#evil`,
    url.replace('/17', '/18'),
    url.replace('create', 'cancel'),
    url.replace('core.example', 'evil.example'),
    url.replace('https:', 'http:'),
    'javascript:alert(1)',
    'https://user@core.example/core/v1/wallet/confirm/create/17',
  ]) {
    assert.equal(
      trustedConfirmationURL(
        raw,
        'create',
        17,
        'https://web.example',
        'https://core.example'
      ),
      null
    )
  }
  const local = 'http://127.0.0.1:8080/core/v1/wallet/confirm/create/17'
  assert.equal(
    trustedConfirmationURL(
      local,
      'create',
      17,
      'http://127.0.0.1:3000',
      'http://127.0.0.1:8080'
    ),
    local
  )
})
test('confirmation preserves original key; only actual state resolves operation', () => {
  const result = reconcileTransfer(pending, {
    kind: 'confirmation',
    confirmationURL: 'https://core.example/core/v1/wallet/confirm/create/17',
    transfer: row,
  })
  assert.equal(result.kind, 'pending')
  if (result.kind === 'pending') {
    assert.equal(result.pending.requestKey, pending.requestKey)
    assert.equal(result.pending.step.kind, 'confirmation')
  }
  assert.equal(
    reconcileTransfer(pending, { kind: 'transfer', transfer: row }).kind,
    'pending'
  )
  assert.equal(
    reconcileTransfer(pending, {
      kind: 'transfer',
      transfer: { ...row, status: 'pending' },
    }).kind,
    'complete'
  )
  assert.throws(() =>
    reconcileTransfer(pending, {
      kind: 'transfer',
      transfer: { ...row, quota: 126 },
    })
  )
  const cancel: PendingTransfer = {
    action: 'cancel',
    id: 17,
    quota: 125,
    step: { kind: 'unknown' },
  }
  assert.equal(
    reconcileTransfer(cancel, {
      kind: 'transfer',
      transfer: { ...row, status: 'pending' },
    }).kind,
    'pending'
  )
  assert.equal(
    reconcileTransfer(cancel, {
      kind: 'transfer',
      transfer: { ...row, status: 'cancelled' },
    }).kind,
    'complete'
  )
  assert.equal(
    reconcileTransfer(cancel, {
      kind: 'transfer',
      transfer: { ...row, status: 'claimed' },
    }).kind,
    'conflict'
  )
})
test('unavailable link and recipient flags hide even stray payload fields', () => {
  const value = { ...row, status: 'pending' as const, token: 'a'.repeat(64) }
  assert.equal(canShareTransfer(value), true)
  assert.equal(
    canShareTransfer({ ...value, claim_link_available: false }),
    false
  )
  assert.equal(canShareTransfer({ ...value, token: undefined }), false)
  assert.equal(
    canShareTransfer({ ...value, status: 'awaiting_confirmation' }),
    false
  )
  assert.equal(
    hasRecipientDetails({ ...value, recipient_id: 5, recipient_name: 'name' }),
    true
  )
  assert.equal(
    hasRecipientDetails({
      ...value,
      recipient_id: 5,
      recipient_name: 'name',
      recipient_details_available: false,
    }),
    false
  )
  assert.equal(hasRecipientDetails(value), false)
})
test('202 is confirmation; create POST disables authentication replay', async () => {
  api.post = (async (
    url: string,
    body: unknown,
    options: Record<string, unknown>
  ) => {
    assert.equal(url, '/api/wallet-transfer')
    assert.deepEqual(body, { quota: 125, request_key: row.request_key })
    assert.equal(options.skipAuthRefresh, true)
    return {
      data: {
        success: false,
        code: 'WALLET_CONFIRMATION_REQUIRED',
        data: {
          request_key: row.request_key,
          transfer: row,
          confirmation_url:
            'https://core.example/core/v1/wallet/confirm/create/17',
        },
      },
    }
  }) as typeof api.post
  assert.equal(
    (await createTransfer(scope, 125, 'immutable-wallet-key')).kind,
    'confirmation'
  )
})
test('lost create ACK only queries exact original key, never dispatches again', async () => {
  let writes = 0
  api.post = (async () => {
    writes++
    throw new Error('lost ACK')
  }) as typeof api.post
  await assert.rejects(createTransfer(scope, 125, 'immutable-wallet-key'))
  api.get = (async (url: string, options: Record<string, unknown>) => {
    assert.equal(url, '/api/wallet-transfer/query')
    assert.deepEqual(options.params, { request_key: row.request_key })
    return { data: { success: true, data: { ...row, status: 'pending' } } }
  }) as typeof api.get
  assert.equal(
    (await queryTransfer(scope, 'immutable-wallet-key')).transfer.status,
    'pending'
  )
  assert.equal(writes, 1)
  api.get = (async () => ({
    data: { success: false, code: 'WALLET_RESULT_UNKNOWN' },
  })) as typeof api.get
  await assert.rejects(queryTransfer(scope, 'immutable-wallet-key'))
  assert.equal(writes, 1)
})
test('lost cancel ACK only queries original ID and pending is still confirmation', async () => {
  let writes = 0
  api.post = (async () => {
    writes++
    throw new Error('lost ACK')
  }) as typeof api.post
  await assert.rejects(cancelTransfer(scope, 17, 125))
  api.get = (async (url: string, options: Record<string, unknown>) => {
    assert.equal(url, '/api/wallet-transfer/query')
    assert.deepEqual(options.params, { cancel_transfer_id: 17 })
    return {
      data: {
        success: false,
        code: 'WALLET_CONFIRMATION_REQUIRED',
        data: {
          request_key: row.request_key,
          transfer: { ...row, status: 'pending' },
          confirmation_url:
            'https://core.example/core/v1/wallet/confirm/cancel/17',
        },
      },
    }
  }) as typeof api.get
  assert.equal((await queryTransferByID(scope, 17)).kind, 'confirmation')
  assert.equal(writes, 1)
})

test('completed cancellation accepts the Extensions null acknowledgement', async () => {
  api.post = (async (
    _url: string,
    _body: unknown,
    options: Record<string, unknown>
  ) => {
    assert.deepEqual(options.authScope, scope)
    assert.equal(options.skipAuthRefresh, true)
    return { data: { success: true, data: null } }
  }) as typeof api.post
  assert.deepEqual(await cancelTransfer(scope, 17, 125), { kind: 'cancelled' })
})

test('transfer readiness requires exact native balance and rejects stale account scope before dispatch', async () => {
  let calls = 0
  api.get = (async (url: string, options: Record<string, unknown>) => {
    calls++
    assert.equal(url, '/api/wallet/balance')
    assert.deepEqual(options.authScope, scope)
    return {
      data: {
        success: true,
        data: {
          schema_version: 2,
          currency_unit: 'CREDIT',
          credits_per_usd: 500000,
          public_credits_per_usd_exact: '500000',
          spendable_credits: 125,
        },
      },
    }
  }) as typeof api.get
  assert.equal((await getTransferBalance(scope)).spendable_credits, 125)
  assert.throws(() => getTransferBalance({ ...scope, userId: 53 }))
  assert.equal(calls, 1)
})

test('claim and inspection bind the original session and reject replies after an account switch', async () => {
  let calls = 0
  api.post = (async (
    url: string,
    body: unknown,
    options: Record<string, unknown>
  ) => {
    calls++
    assert.deepEqual(options.authScope, scope)
    assert.equal(options.skipAuthRefresh, true)
    assert.deepEqual(body, { token: 'a'.repeat(64) })
    if (url.endsWith('/claim')) {
      useAuthStore
        .getState()
        .auth.setUser({ id: 53, role: 1, username: 'replacement' })
    }
    return {
      data: {
        success: true,
        data: {
          quota: 125,
          status: 'pending',
          created_at: 1,
          is_sender: false,
          claimed_by_me: false,
        },
      },
    }
  }) as typeof api.post
  assert.equal((await inspectTransfer(scope, 'a'.repeat(64))).quota, 125)
  await assert.rejects(claimTransfer(scope, 'a'.repeat(64)))
  assert.throws(() => inspectTransfer(scope, 'a'.repeat(64)))
  assert.equal(calls, 2, 'a stale scope must not dispatch another request')
})

test('claim success requires a claimed receipt for this recipient', async () => {
  let claimed = false
  api.post = (async () => ({
    data: {
      success: true,
      data: {
        quota: 125,
        status: claimed ? 'claimed' : 'pending',
        created_at: 1,
        is_sender: false,
        claimed_by_me: claimed,
      },
    },
  })) as typeof api.post
  await assert.rejects(claimTransfer(scope, 'a'.repeat(64)))
  claimed = true
  assert.equal((await claimTransfer(scope, 'a'.repeat(64))).claimed_by_me, true)
})

test('restored request keys preserve Core UTF-8 byte limits and historical content', () => {
  for (const requestKey of [
    `${'界'.repeat(5)}a`,
    `${'界'.repeat(21)}a`,
    'a'.repeat(64),
  ]) {
    assert.equal(
      pendingSchema.parse({ ...pending, requestKey }).requestKey,
      requestKey
    )
    assert.equal(
      pendingSchema.parse({
        action: 'cancel',
        id: 17,
        quota: 125,
        requestKey,
        step: { kind: 'unknown' },
      }).requestKey,
      requestKey
    )
  }
  for (const requestKey of [
    '界'.repeat(5),
    '界'.repeat(22),
    'a'.repeat(65),
    ` ${'a'.repeat(16)}`,
    `\u0085${'a'.repeat(16)}`,
    `${'a'.repeat(16)}\0`,
    `${'a'.repeat(16)}\uD800`,
  ]) {
    assert.equal(
      pendingSchema.safeParse({ ...pending, requestKey }).success,
      false
    )
  }
})
