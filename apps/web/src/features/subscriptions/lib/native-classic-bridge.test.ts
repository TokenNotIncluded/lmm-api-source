/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import type { NativePlan, NativePurchase } from '../native-api'
import {
  nativePlanToRecord,
  nativePurchasesToSelfData,
} from './native-classic-bridge'

const plan: NativePlan = {
  id: 4,
  revision: 1,
  archived: false,
  terms: {
    title: 'Credit-only plan',
    price_units: '100001',
    duration_seconds: 3600,
    reset_period: 'grant',
    reset_seconds: 0,
    limit_units: '170003',
    models: ['*'],
    groups: ['*'],
    max_purchases_per_user: 0,
    enabled: true,
    allow_balance_pay: true,
  },
}

test('native plans map into classic card records with balance checkout', () => {
  const record = nativePlanToRecord(plan)
  assert.equal(record.contract, 'core.subscriptions.v1')
  assert.equal(record.plan.title, 'Credit-only plan')
  assert.equal(record.balance_price_quota, 100001)
  assert.equal(record.plan.total_amount, 170003)
  assert.deepEqual(record.payment_methods, ['balance'])
  assert.equal(record.plan.duration_unit, 'hour')
  assert.equal(record.plan.duration_value, 1)
})

test('native purchases map into my-subscription rows for the classic card', () => {
  const purchase: NativePurchase = {
    id: 9,
    subscription_id: 6,
    plan,
    starts_at: 1,
    ends_at: 2,
    used_units: '170003',
    reserved_units: '0',
    entitlement_active: false,
    renewal_canceled: false,
    canceled_at: null,
    refund_journal_id: null,
  }
  const self = nativePurchasesToSelfData([purchase], 7, 10)
  assert.equal(self.contract, 'core.subscriptions.v1')
  assert.equal(self.subscriptions.length, 0)
  assert.equal(self.all_subscriptions.length, 1)
  assert.equal(self.all_subscriptions[0].subscription.id, 6)
  assert.equal(self.all_subscriptions[0].subscription.status, 'expired')
  assert.equal(self.all_subscriptions[0].subscription.amount_used, 170003)
  assert.throws(
    () =>
      nativePurchasesToSelfData(
        [{ ...purchase, used_units: '9007199254740993' }],
        7,
        10
      ),
    /exceeds the classic projection/
  )
})
