import assert from 'node:assert/strict'
/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { test } from 'node:test'

import { parse } from 'lossless-json'

import { batchQuoteRow } from './official-batch-price-review'
import { readOfficialToml } from './official-price-toml'

test('batch prices require an exact model and an unambiguous provider, and keep absent prices missing', async () => {
  const catalog = parse(
    '{"a":{"models":{"m":{"cost":{"input":2,"output":3,"cache_read":0}}}},"b":{"models":{"m":{"cost":{"input":7}}}}}'
  )
  const ambiguous = await batchQuoteRow(catalog, 'm')
  assert.deepEqual(ambiguous.providers, ['a', 'b'])
  assert.equal(ambiguous.toml, '')
  const mapped = await batchQuoteRow(catalog, 'public-alias', 'm', 'a')
  const price = readOfficialToml('public-alias', mapped.toml)
  assert.equal(price.rates.input, 2_000_000)
  assert.equal(price.rates.cache_read, 0)
  assert.equal(price.rates.cache_write, undefined)
  assert.equal(price.rates.request, undefined)
  assert.equal(price.source, 'models.dev/a/m')
  assert.equal((await batchQuoteRow(catalog, 'M')).selected, false)
})
