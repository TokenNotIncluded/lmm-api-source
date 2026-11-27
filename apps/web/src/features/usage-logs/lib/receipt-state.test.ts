/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { usageLogStatusCode, usageReceiptState } from './receipt-state'

test('incomplete native receipts identify reconciliation separately', () => {
  assert.deepEqual(
    usageReceiptState({ billing_basis: 'incomplete', state: 'reconcile' }),
    { status: 'reconcile', label: 'Reconciling', variant: 'warning' }
  )
  for (const state of ['reserved', 'streaming', 'released', undefined]) {
    assert.deepEqual(
      usageReceiptState({ billing_basis: 'incomplete', state }),
      { status: 'incomplete', label: 'Incomplete', variant: 'warning' }
    )
  }
})

test('metadata-only incomplete basis uses its captured request state', () => {
  assert.equal(
    usageReceiptState(
      {},
      { billing_basis: 'incomplete', request_state: 'reconcile' }
    )?.status,
    'reconcile'
  )
  assert.equal(
    usageReceiptState(
      { state: 'streaming' },
      { billing_basis: 'incomplete', request_state: 'reconcile' }
    )?.status,
    'incomplete'
  )
})

test('state names and legacy errors do not invent a native incomplete receipt', () => {
  for (const state of [undefined, 'reconcile', 'reserved', 'settled']) {
    assert.equal(usageReceiptState({ state }), undefined)
  }
  assert.equal(
    usageReceiptState(
      { billing_basis: 'net_consumption', state: 'settled' },
      { billing_basis: 'net_consumption', request_state: 'settled' }
    ),
    undefined
  )
})

test('native HTTP status takes priority over historical metadata', () => {
  assert.equal(
    usageLogStatusCode(
      { status_code: 503, provider_http_status: 404 },
      { status_code: 400 }
    ),
    503
  )
  assert.equal(
    usageLogStatusCode({ provider_http_status: 404 }, { status_code: 400 }),
    404
  )
  assert.equal(usageLogStatusCode({}, { status_code: 400 }), 400)
  assert.equal(usageLogStatusCode({}, null), undefined)
  assert.equal(usageLogStatusCode({}, { status_code: 999 }), undefined)
})
