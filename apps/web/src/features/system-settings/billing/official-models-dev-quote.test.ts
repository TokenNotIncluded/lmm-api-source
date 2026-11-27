/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { parse } from 'lossless-json'

import {
  fetchOfficialModelsDevQuote,
  officialModelsDevQuote,
} from './official-models-dev-quote'

const quote = (cost: string) =>
  officialModelsDevQuote(
    parse(`{"p":{"models":{"m":{"cost":${cost}}}}}`),
    'm',
    'p'
  )

test('source quotes preserve units, exact prices, zeros and unknown meters', () => {
  const expected = {
    model: 'm',
    source: { kind: 'models_dev', provider: 'p' },
    currency: 'USD',
    token_unit: 'per_million_tokens',
    source_token_unit: 'per_million_tokens',
    rates: {
      input: '0.01234567890123456789',
      cache_read: '0',
      reasoning: '0.000003',
    },
    context_tiers: null,
  }
  assert.deepEqual(
    quote(
      '{"input":0.01234567890123456789,"cache_read":0,"reasoning":3e-6,"output":null}'
    ),
    expected
  )
  assert.equal(quote('{}'), null)
  assert.throws(
    () => quote('{"input":1,"cost_per_hour":8}'),
    /unsupported_source_pricing/
  )
  assert.throws(() => quote('{"input":"0.1"}'), /invalid_source_price/)
})

test('context tiers retain explicit thresholds and sparse fields, not the legacy 200k alias', () => {
  assert.deepEqual(
    quote(
      '{"input":1,"tiers":[{"tier":{"type":"context","size":272000},"input":4}],"context_over_200k":{"input":4}}'
    )?.context_tiers,
    [{ above_input_tokens: 272000, rates: { input: '4' } }]
  )
  assert.deepEqual(quote('{"input":1,"tiers":[]}')?.context_tiers, [])
  assert.throws(
    () => quote('{"input":1,"context_over_200k":{"input":4}}'),
    /missing_context_threshold/
  )
  assert.throws(
    () =>
      quote(
        '{"tiers":[{"tier":{"type":"context","size":9007199254740993},"input":4}]}'
      ),
    /invalid_context_threshold/
  )
  assert.throws(
    () => quote('{"tiers":[{"tier":{"type":"time","size":200},"input":4}]}'),
    /unsupported_context_condition/
  )
})

test('direct sync fetches the selected model and provider through the public catalog', async () => {
  const body =
    '{"p":{"models":{"m":{"cost":{"output":5}}}},"alt":{"models":{"m":{"cost":{"input":2}}}}}'
  const saved = globalThis.fetch
  globalThis.fetch = async (url, init) => {
    assert.equal(url, 'https://models.dev/api.json')
    assert.equal(init?.credentials, 'omit')
    assert.equal(init?.redirect, 'error')
    assert.ok(init?.signal instanceof AbortSignal)
    return new Response(body, {
      headers: { 'content-type': 'application/json' },
    })
  }
  try {
    assert.deepEqual((await fetchOfficialModelsDevQuote('m', 'alt'))?.rates, {
      input: '2',
    })
    assert.equal(await fetchOfficialModelsDevQuote('M', 'p'), null)
    assert.equal(await fetchOfficialModelsDevQuote('m', 'toString'), null)
  } finally {
    globalThis.fetch = saved
  }
})
