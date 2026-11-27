/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { nativeConfiguredPrice } from '@/features/channels/lib/native-price-display'
import { stringifyNativeDocument } from '@/features/channels/native-api'

import {
  decimalPriceFraction,
  fractionText,
  priceChanges,
  priceRateText,
  readPriceDocument,
  readPriceRules,
} from './native-price-rules'

function rule() {
  return {
    id: 19,
    model: 'example-model',
    group: 'default',
    speed: 'standard' as const,
    rates: {
      input: 2_000_001,
      output: 6_000_000,
      cache_read: 500_000,
      cache_write: 0,
      request: 1,
    },
    group_multiplier_ppm: 750_000,
    speed_multiplier_ppm: 1_000_000,
  }
}
test('price editing preserves exact rates, rounding and tariff fields', () => {
  const original = { ...rule(), rounding: 'legacy_go_ratio_half_away_min1_v1' }
  const prices = readPriceDocument(
    JSON.stringify({ version: 2, rules: [original] }),
    3
  )
  assert.equal(prices.version, 3)
  assert.deepEqual(prices.rules[0], original)
  assert.equal(priceRateText(rule(), 'input'), '2.000001')
  assert.equal(priceRateText(rule(), 'request'), '0.000001')
})
test('duplicate selectors and unsafe integers are rejected before preview', () => {
  assert.throws(
    () =>
      readPriceRules({ version: 2, rules: [rule(), { ...rule(), id: 20 }] }),
    /unique/
  )
  assert.throws(() =>
    readPriceRules({
      version: 2,
      rules: [
        {
          ...rule(),
          rates: { ...rule().rates, input: Number.MAX_SAFE_INTEGER + 1 },
        },
      ],
    })
  )
})
test('tiered and upstream-cost prices never display placeholder zero rates', () => {
  const fraction = decimalPriceFraction('0.000000000000000001')
  assert.ok(fraction)
  const tiered = {
    ...rule(),
    billing_mode: 'tiered_decimal' as const,
    tiered: { tiers: [{ rates: { input: fraction } }] },
  }
  assert.equal(priceRateText(tiered, 'input'), '0.000000000000000001')
  assert.equal(priceRateText(tiered, 'request'), null)
  assert.equal(
    priceRateText({ ...rule(), billing_mode: 'upstream_cost' }, 'input'),
    null
  )
  assert.match(
    stringifyNativeDocument(tiered),
    /"denominator":1000000000000000000/
  )
  assert.equal(fractionText(decimalPriceFraction('1/3')), '1/3')
  assert.equal(decimalPriceFraction('1e-6'), null)
  assert.equal(decimalPriceFraction('1/0'), null)
  assert.equal(decimalPriceFraction('-1'), null)
})
test('publication summary accounts for additions, edits and deletions', () => {
  const before = {
    version: 1,
    rules: [rule(), { ...rule(), id: 20, model: 'removed-model' }],
  }
  const after = {
    version: 2,
    rules: [
      { ...rule(), group_multiplier_ppm: 1_000_000 },
      { ...rule(), id: 21, model: 'added-model' },
    ],
  }
  assert.deepEqual(priceChanges(before, after), {
    added: 1,
    changed: 1,
    removed: 1,
  })
  assert.deepEqual(
    priceChanges(
      { version: 1, rules: [rule()] },
      {
        version: 2,
        rules: [Object.fromEntries(Object.entries(rule()).reverse())],
      }
    ),
    { added: 0, changed: 0, removed: 0 }
  )
})

test('published official flat rates survive document round trips and display exact source and effective prices', () => {
  const base = rule()
  const rate = (numerator: number) => ({ numerator, denominator: 1_000_000 })
  const source = {
    ...base,
    billing_mode: 'official_flat' as const,
    rates: { input: 0, output: 0, cache_read: 0, cache_write: 0, request: 0 },
    group_multiplier_ppm: 750_000,
    speed_multiplier_ppm: 1_200_000,
    tiered: {
      revision: 2,
      source_digest: 'ab'.repeat(32),
      uses_cache_read: true,
      uses_cache_write: true,
      uses_cache_write_1h: true,
      tiers: [
        {
          up_to_input_tokens: null,
          rates: {
            input: rate(2_000_001),
            output: rate(6_000_000),
            cache_read: rate(0),
            cache_write: rate(500_000),
            cache_write_1h: rate(500_000),
          },
        },
      ],
    },
  }
  const published = { version: 2, rules: [source] }
  const read = readPriceDocument(stringifyNativeDocument(published), 2)
  const [decoded] = readPriceRules(read)
  assert.deepEqual(decoded, source)
  assert.deepEqual(
    readPriceDocument(stringifyNativeDocument(read), 2),
    published
  )
  assert.equal(priceRateText(decoded, 'input'), '2.000001')
  assert.equal(priceRateText(decoded, 'cache_read'), '0')
  assert.equal(priceRateText(decoded, 'cache_write'), '0.5')
  assert.equal(priceRateText(decoded, 'request'), null)
  assert.equal(nativeConfiguredPrice(decoded, 'input'), '1.8000009')
  assert.equal(nativeConfiguredPrice(decoded, 'cache_write'), '0.45')
  assert.equal(nativeConfiguredPrice(decoded, 'request'), null)
})
