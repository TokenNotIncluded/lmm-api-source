/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { LosslessNumber } from 'lossless-json'

import { publishedPriceBands } from './native-price-bands'
import { type NativePriceRule } from './native-price-rules'

const scalar: NativePriceRule = {
  id: 1,
  model: 'm',
  group: 'g',
  speed: 'standard',
  group_multiplier_ppm: 250_000,
  speed_multiplier_ppm: 2_000_000,
  rates: { input: 1, output: 2, cache_read: 0, cache_write: 0, request: 5 },
}
const rate = (numerator: number | LosslessNumber, denominator = 1) => ({
  numerator,
  denominator,
})

test('scalar prices preserve micro-USD per unit and explicit zeros', () => {
  const [band] = publishedPriceBands(scalar).bands
  assert.deepEqual(band.prices[0], {
    meter: 'input',
    state: 'rate',
    base: '0.000001',
    final: '0.0000005',
  })
  assert.equal(
    band.prices.find((p) => p.meter === 'request')?.final,
    '0.0000025'
  )
  assert.equal(band.prices.find((p) => p.meter === 'cache_read')?.final, '0')
  assert.equal(
    band.prices.find((p) => p.meter === 'cache_write_12h')?.state,
    'unused'
  )
  assert.equal(
    publishedPriceBands({ ...scalar, billing_mode: 'upstream_cost' }).bands
      .length,
    0
  )
})

test('ordinary and calendar tiers retain their own exact rates, bounds and missing dimensions', () => {
  const a = {
    input: rate(1, 3),
    output: rate(1),
    cache_read: rate(0),
    cache_write: rate(2),
    cache_write_12h: rate(3),
  }
  const b = {
    input: rate(new LosslessNumber('9007199254740993')),
    output: rate(2),
    cache_write: rate(6),
  }
  const rule: NativePriceRule = {
    ...scalar,
    billing_mode: 'tiered_decimal',
    tiered: {
      revision: 4,
      uses_cache_read: true,
      uses_cache_write: true,
      uses_cache_write_12h: true,
      tiers: [{ up_to_input_tokens: 200000, rates: a }, { rates: b }],
      calendar: { match_tiers: [{ rates: { ...a, input: rate(7) } }] },
    },
  }
  const { bands } = publishedPriceBands(rule)
  assert.equal(bands.length, 3)
  assert.deepEqual(
    bands.map((b) => [b.lower, b.upper, b.branch]),
    [
      [null, '200000', 'standard'],
      ['200000', null, 'standard'],
      [null, null, 'calendar'],
    ]
  )
  assert.deepEqual(
    bands.map((b) => b.prices[0].final),
    ['1/6', '4503599627370496.5', '3.5']
  )
  assert.equal(
    bands[1].prices.find((p) => p.meter === 'cache_write_12h')?.state,
    'missing'
  )
  assert.equal(
    bands[0].prices.find((p) => p.meter === 'request')?.state,
    'unused'
  )
})
