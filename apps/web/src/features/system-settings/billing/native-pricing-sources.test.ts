/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { parse } from 'lossless-json'

import type { NativeConfiguration } from '@/features/channels/native-api'

import {
  configuredPriceSpeeds,
  routingPriceCoverage,
} from './native-price-coverage'
import {
  compileLinearExpression,
  expressionPriceRule,
  modelsDevPrices,
} from './native-pricing-sources'

test('public catalog requires an exact model ID and keeps USD-per-million scale', () => {
  const document = parse(
    '{"one":{"name":"Provider One","models":{"precise":{"cost":{"input":1.25,"output":10,"cache_read":0.125}}}},"missing":{"models":{"precise":{"cost":{"input":0}}}},"round":{"models":{"precise":{"cost":{"input":0.0000001,"output":0}}}}}'
  )
  assert.deepEqual(modelsDevPrices(document, 'precise'), [
    {
      provider: 'one',
      name: 'Provider One',
      rates: { input: 1250000, output: 10000000, cache_read: 125000 },
    },
  ])
  assert.deepEqual(modelsDevPrices(document, 'PrecisE'), [])
})

test('linear expression coefficients stay exact and zero cache terms remain explicit', () => {
  assert.deepEqual(
    compileLinearExpression(
      'tier("standard", p * 1.25 + c*10 + cc*0 + cr*0.125)'
    ),
    {
      revision: 2,
      uses_cache_read: true,
      uses_cache_write: true,
      tiers: [
        {
          up_to_input_tokens: null,
          rates: {
            input: { numerator: 5, denominator: 4 },
            output: { numerator: 10, denominator: 1 },
            cache_read: { numerator: 1, denominator: 8 },
            cache_write: { numerator: 0, denominator: 1 },
          },
        },
      ],
    }
  )
  for (const unsupported of [
    'tier("standard", p * -1 + c * 2)',
    'tier("standard", p * 1 + c * 2 + c*3)',
    'tier("standard", p * 1 + c * 1e2)',
    'tier("standard", p * 1 + c * 2 + max(p, 3))',
    'tier("standard", p * 1 + c * 2 + aux * 3)',
    'p * 1 + c * 2',
  ]) {
    assert.equal(compileLinearExpression(unsupported), null)
  }
})

test('expression compilation changes a local rule only and preserves selector and multipliers', async () => {
  const rule = {
    id: 1,
    model: 'model',
    group: 'default',
    speed: 'standard' as const,
    group_multiplier_ppm: 2_000_000,
    speed_multiplier_ppm: 1_000_000,
    rates: { input: 1, output: 2, cache_read: 3, cache_write: 4, request: 0 },
  }
  assert.equal(
    await expressionPriceRule(
      { ...rule, rates: { ...rule.rates, request: 1 } },
      'tier("base",p*1+c*2)'
    ),
    null
  )
  const compiled = await expressionPriceRule(rule, 'tier("base",p*1+c*2)')
  assert.ok(compiled)
  assert.equal(compiled.group_multiplier_ppm, rule.group_multiplier_ppm)
  assert.equal(compiled.model, rule.model)
  assert.equal(compiled.billing_mode, 'tiered_decimal')
  assert.equal(rule.rates.input, 1)
  assert.match(
    String((compiled.tiered as { source_digest: string }).source_digest),
    /^[a-f0-9]{64}$/
  )
})

test('price selectors exclude premium tiers even when route and targets request them', () => {
  const routing = {
    groups: [
      { name: 'default', enabled: true },
      { name: 'off', enabled: false },
    ],
    upstreams: [
      { id: 1, enabled: true },
      { id: 2, enabled: false },
    ],
    routes: [
      {
        model: 'one',
        group: 'default',
        enabled: true,
        speeds: ['standard', 'fast', 'ultrafast'],
        targets: [
          { upstream_id: 1, enabled: true, speeds: ['standard'] },
          { upstream_id: 1, enabled: true, speeds: ['fast'] },
          { upstream_id: 2, enabled: true, speeds: ['ultrafast'] },
        ],
      },
      {
        model: 'two',
        group: 'default',
        enabled: false,
        speeds: ['fast'],
        targets: [],
      },
    ],
  } as NativeConfiguration['routing']
  assert.deepEqual(configuredPriceSpeeds(routing, 'one', 'default'), [
    'standard',
  ])
  assert.deepEqual(configuredPriceSpeeds(routing, 'one', 'off'), [])
  assert.deepEqual(configuredPriceSpeeds(routing, 'two', 'default'), [])
  assert.deepEqual(routingPriceCoverage(routing, []).missing, [
    { model: 'one', group: 'default', speed: 'standard' },
  ])
})
