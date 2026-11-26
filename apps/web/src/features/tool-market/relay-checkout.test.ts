/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { formatRelayUSD, readRelayOrder, relayOrderLink } from './relay-order'

test('relay checkout validates the server response against the displayed user, order and price', () => {
  const data = {
    order_id: 'a'.repeat(64),
    user_id: 42,
    gib: 2,
    amount_usd_cents: 200,
    period: '2026-11',
    state: 'preview',
  }
  assert.equal(
    readRelayOrder({ success: true, data }, data.order_id, 42).gib,
    2
  )
  for (const change of [
    { user_id: 41 },
    { order_id: 'b'.repeat(64) },
    { amount_usd_cents: 3 },
    { state: 'unrecognized' },
  ]) {
    assert.throws(() =>
      readRelayOrder(
        { success: true, data: { ...data, ...change } },
        data.order_id,
        42
      )
    )
  }
  assert.throws(() =>
    readRelayOrder({ success: false, data }, data.order_id, 42)
  )
})

test('relay checkout honors a frozen configurable rate, including sub-cent totals', () => {
  const data = {
    order_id: 'c'.repeat(64),
    user_id: 7,
    gib: 3,
    period: '2026-11',
    state: 'preview',
    unit_price_quota_per_gib: 250001,
    total_price_quota: 750003,
  }
  const read = (change = {}) =>
    readRelayOrder(
      { success: true, data: { ...data, ...change } },
      data.order_id,
      7
    )
  assert.equal(read().total_price_quota, 750003)
  assert.equal(formatRelayUSD(read().total_price_quota, 'en-US'), '$1.500006')
  assert.equal(formatRelayUSD(1, 'en-US'), '$0.000002')
  assert.equal(formatRelayUSD(500000, 'en-US'), '$1.00')
  assert.equal(
    formatRelayUSD(Number.MAX_SAFE_INTEGER, 'en-US'),
    '$18,014,398,509.481982'
  )
  for (const change of [
    { total_price_quota: 750004 },
    { unit_price_quota_per_gib: 0 },
    { unit_price_quota_per_gib: undefined },
    { total_price_quota: undefined },
    { total_price_quota: Number.MAX_SAFE_INTEGER + 1 },
    { amount_usd_cents: 150 },
  ]) {
    assert.throws(() => read(change))
  }
})

test('only the compiled purchase tool links a successful order to the local checkout', () => {
  const id = '3'.repeat(64),
    tool = '1ae22cbb-5d64-5b9d-95b2-d2ac5f934682'
  const preview = {
    structuredContent: {
      order: { order_id: id },
      confirmation_path: 'https://external.invalid/ignore',
    },
  }
  assert.equal(
    relayOrderLink(tool, 'succeeded', preview),
    '/tool-market?relay_order=' + id
  )
  assert.equal(relayOrderLink('another-tool', 'succeeded', preview), undefined)
  assert.equal(relayOrderLink(tool, 'failed', preview), undefined)
  assert.equal(
    relayOrderLink(tool, 'succeeded', { ...preview, isError: true }),
    undefined
  )
  assert.equal(
    relayOrderLink(tool, 'succeeded', {
      structuredContent: { order: { order_id: '../outside' } },
    }),
    undefined
  )
})
