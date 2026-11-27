/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { LosslessNumber } from 'lossless-json'

import { nativeConfiguredPrice } from '../native-price-display'

const multipliers = {
  group_multiplier_ppm: 1_000_000,
  speed_multiplier_ppm: 1_000_000,
}
function tiered(numerator: unknown, denominator: unknown) {
  return {
    ...multipliers,
    billing_mode: 'tiered_decimal',
    tiered: { tiers: [{ rates: { input: { numerator, denominator } } }] },
  }
}

test('configured micro-USD prices include both multipliers without rounding', () => {
  const rule = {
    group_multiplier_ppm: 1_500_000,
    speed_multiplier_ppm: 500_000,
    rates: { input: 2_500_000, output: 5_000_000, request: 125_000 },
  }
  assert.equal(nativeConfiguredPrice(rule, 'input'), '1.875')
  assert.equal(nativeConfiguredPrice(rule, 'output'), '3.75')
  assert.equal(nativeConfiguredPrice(rule, 'request'), '0.09375')
  assert.equal(
    nativeConfiguredPrice({ ...rule, group_multiplier_ppm: 0 }, 'input'),
    '0'
  )
  assert.equal(
    nativeConfiguredPrice({ ...multipliers, rates: { input: 0 } }, 'input'),
    '0'
  )
})

test('tiered catalog prices retain large integers and exact recurring fractions', () => {
  assert.equal(
    nativeConfiguredPrice(
      tiered(
        new LosslessNumber('9007199254740993'),
        new LosslessNumber('1000000000000000000')
      ),
      'input'
    ),
    '0.009007199254740993'
  )
  assert.equal(nativeConfiguredPrice(tiered(2, 6), 'input'), '1/3')
  assert.equal(nativeConfiguredPrice(tiered(0, 6), 'input'), '0')
  const rule = tiered(1, 2)
  rule.tiered.tiers.push({ rates: { input: { numerator: 3, denominator: 4 } } })
  assert.equal(nativeConfiguredPrice(rule, 'input', 1), '0.75')
  assert.equal(nativeConfiguredPrice(rule, 'input', 2), null)
})

test('conditional, upstream-cost and malformed prices never become an invented zero', () => {
  const rule = { ...multipliers, rates: { input: 0 } }
  for (const billing_mode of ['upstream_cost', 'unsupported']) {
    assert.equal(
      nativeConfiguredPrice({ ...rule, billing_mode }, 'input'),
      null
    )
  }
  const conditional = tiered(0, 1)
  assert.equal(
    nativeConfiguredPrice(
      {
        ...conditional,
        tiered: { ...conditional.tiered, calendar: { timezone: 'UTC' } },
      },
      'input'
    ),
    null
  )
  assert.equal(
    nativeConfiguredPrice({ ...rule, tiered: conditional.tiered }, 'input'),
    null
  )
  assert.equal(nativeConfiguredPrice({ rates: { input: 0 } }, 'input'), null)
  assert.equal(nativeConfiguredPrice({ ...rule, rates: {} }, 'input'), null)
  for (const numerator of [null, -1, 0.5, Number.MAX_SAFE_INTEGER + 1, '0']) {
    assert.equal(nativeConfiguredPrice(tiered(numerator, 1), 'input'), null)
  }
  assert.equal(nativeConfiguredPrice(tiered(1, 0), 'input'), null)
  assert.equal(nativeConfiguredPrice(tiered(1, -1), 'input'), null)
})
