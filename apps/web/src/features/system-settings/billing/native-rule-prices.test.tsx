/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { renderToStaticMarkup } from 'react-dom/server'

import type { NativeConfiguration } from '@/features/channels/native-api'

import type { NativePriceRule } from './native-price-rules'
import { NativeRulePrices } from './native-rule-prices'

test('draft and published rates keep their own amounts and provenance', () => {
  const rule: NativePriceRule = {
    id: 1,
    model: 'm',
    group: 'g',
    speed: 'standard',
    rates: {
      input: 1000000,
      output: 2000000,
      cache_read: 0,
      cache_write: 0,
      request: 0,
    },
    group_multiplier_ppm: 1000000,
    speed_multiplier_ppm: 1000000,
  }
  const config = {
    version: { routing: 3, pricing: 8 },
    prices: { version: 8, rules: [rule] },
    routing: {
      version: 3,
      groups: [{ name: 'g', enabled: true }],
      routes: [],
      upstreams: [],
    },
    bindings: {},
  } as unknown as NativeConfiguration
  const html = renderToStaticMarkup(
    <NativeRulePrices
      rule={{ ...rule, group_multiplier_ppm: 125000 }}
      current={rule}
      configuration={config}
      cached
    />
  )
  for (const expected of [
    'Draft unit rate',
    'Published unit rate',
    '0.125',
    '0.25',
    'Not recorded',
    'Last loaded version',
    'No enabled target',
  ]) {
    assert.ok(html.includes(expected), expected)
  }
  assert.ok(!html.includes('models.dev'))
})
