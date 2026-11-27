/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { LosslessNumber } from 'lossless-json'

import type { NativeConfiguration } from '@/features/channels/native-api'

import type { OfficialSourceQuote } from './official-models-dev-quote'
import { proposedPublication, officialModelChanges } from './official-price-api'
import {
  completeOfficialPrice,
  mergeQuote,
  missingOfficialRatePaths,
  readOfficialToml,
  writeOfficialToml,
  type OfficialPrice,
} from './official-price-toml'

const base: OfficialPrice = {
  source: 'manual',
  source_unit: 'USD/1M tokens',
  rates: {
    input: 1_000_000,
    output: 2_000_000,
    cache_read: 33,
    cache_write: 44,
    request: 0,
  },
}
const quote: OfficialSourceQuote = {
  model: 'a/m',
  source: { kind: 'channel', channel_id: 31 },
  currency: 'USD',
  source_token_unit: 'per_token',
  token_unit: 'per_million_tokens',
  rates: { input: '0.25', cache_read: '0' },
  context_tiers: null,
}
test('a single-model import keeps other prices, multipliers, unrelated bindings, CAS and IDs', () => {
  const config = {
    version: { routing: 7, pricing: 4 },
    routing: { version: 7, groups: [], routes: [] },
    prices: { version: 4, rules: [{ id: 67 }] },
    bindings: {
      custom: { unrelated: 1 },
      official_prices: {
        models: { other: base },
        group_multipliers_ppm: { staff: 1250000 },
        speed_multipliers_ppm: { fast: 1600000, ultrafast: 2400000 },
      },
    },
  } as unknown as NativeConfiguration
  const proposal = proposedPublication(
    config,
    'a/m',
    writeOfficialToml('a/m', base)
  )
  assert.deepEqual(proposal.expected_version, { routing: 7, pricing: 4 })
  assert.equal(proposal.routing.version, 8)
  assert.equal(proposal.prices.version, 5)
  assert.strictEqual(proposal.prices.rules, config.prices.rules)
  const previous = config.bindings.official_prices!,
    next = proposal.bindings.official_prices
  assert.strictEqual(previous.models.other, next.models.other)
  assert.strictEqual(previous.group_multipliers_ppm, next.group_multipliers_ppm)
  assert.strictEqual(previous.speed_multipliers_ppm, next.speed_multipliers_ppm)
  assert.deepEqual(officialModelChanges(previous.models, next.models), {
    added: ['a/m'],
    modified: [],
    deleted: [],
  })
  assert.equal('a/m' in previous.models, false)
  assert.deepEqual(
    officialModelChanges(next.models, {
      other: { ...base, source: 'updated' },
    }),
    { added: [], modified: ['other'], deleted: ['a/m'] }
  )
  assert.throws(() =>
    proposedPublication(config, 'different', writeOfficialToml('a/m', base))
  )
})
test('source refresh preserves absent meters, explicit zero and partial drafts', async () => {
  const doc = await mergeQuote('a/m', writeOfficialToml('a/m', base), quote)
  const value = completeOfficialPrice(readOfficialToml('a/m', doc))
  assert.deepEqual(value.rates, { ...base.rates, input: 250000, cache_read: 0 })
  assert.equal(value.source, 'channel/31')
  assert.equal(value.source_unit, 'USD/token')
  const pending = await mergeQuote('a/m', writeOfficialToml('a/m'), quote)
  assert.deepEqual(readOfficialToml('a/m', pending).rates, {
    input: 250000,
    cache_read: 0,
  })
  assert.throws(() => completeOfficialPrice(readOfficialToml('a/m', pending)))
})
test('long-context prices keep exact fractions and distinct cache rates across TOML round trips', async () => {
  const tiered = await mergeQuote('a/m', writeOfficialToml('a/m', base), {
    ...quote,
    rates: { input: '0.123456789012345678', output: '2' },
    context_tiers: [
      { above_input_tokens: 272000, rates: { input: '4', cache_write: '5' } },
    ],
  })
  const p = completeOfficialPrice(readOfficialToml('a/m', tiered))
  assert.equal(p.tiered?.tiers[0].up_to_input_tokens, 272000)
  assert.equal(
    String(p.tiered?.tiers[0].rates.input.numerator),
    '61728394506172839'
  )
  assert.ok(p.tiered?.tiers[0].rates.input.numerator instanceof LosslessNumber)
  const high = p.tiered!.tiers[1]
  assert.equal(high.up_to_input_tokens, null)
  assert.equal(high.rates.cache_write.numerator, 5)
  const refreshed = completeOfficialPrice(
    readOfficialToml('a/m', await mergeQuote('a/m', tiered, quote))
  )
  assert.deepEqual(refreshed.tiered!.tiers[1], high)
  const other = await mergeQuote('a/m', writeOfficialToml('a/m', base), {
    ...quote,
    rates: { image: '0.03' },
  })
  assert.throws(() => completeOfficialPrice(readOfficialToml('a/m', other)))
})

test('a sub-micro quote uses one exact tier and requires an explicit rate for each meter', async () => {
  const partial = readOfficialToml(
    'a/m',
    await mergeQuote('a/m', writeOfficialToml('a/m'), {
      ...quote,
      rates: {
        input: '0.0000005',
        cache_read: '0',
        cache_write: '2',
        request: '0',
      },
    })
  )
  assert.equal(partial.billing_mode, 'tiered_decimal')
  assert.equal(partial.rates.input, 0)
  const first = partial.tiered?.tiers[0]
  assert.ok(first)
  assert.equal(first.up_to_input_tokens, null)
  assert.deepEqual(first.rates.input, { numerator: 1, denominator: 2000000 })
  assert.deepEqual(first.rates.cache_read, { numerator: 0, denominator: 1 })
  assert.equal(Object.hasOwn(first.rates, 'output'), false)
  assert.throws(() => completeOfficialPrice(partial))
  const completed = completeOfficialPrice(
    readOfficialToml(
      'a/m',
      await mergeQuote('a/m', writeOfficialToml('a/m', partial), {
        ...quote,
        rates: { output: '0' },
      })
    )
  )
  assert.deepEqual(completed.tiered?.tiers[0].rates.output, {
    numerator: 0,
    denominator: 1,
  })
  assert.deepEqual(completed.tiered?.tiers[0].rates.input, first.rates.input)
})

test('free cache meters survive a scalar-to-context import and a later refresh', async () => {
  const flat = {
    ...base,
    rates: { ...base.rates, cache_read: 0, cache_write: 0 },
  }
  const schedule = {
    ...quote,
    rates: { input: '1', output: '2' },
    context_tiers: [{ above_input_tokens: 100000, rates: { input: '1' } }],
  }
  const document = await mergeQuote(
    'a/m',
    writeOfficialToml('a/m', flat),
    schedule
  )
  const price = completeOfficialPrice(readOfficialToml('a/m', document))
  const tariff = price.tiered
  assert.ok(tariff)
  assert.equal(tariff.revision, 2)
  assert.equal(tariff.uses_cache_read, true)
  assert.equal(tariff.uses_cache_write, true)
  for (const band of tariff.tiers) {
    assert.deepEqual(band.rates.cache_read, { numerator: 0, denominator: 1 })
    assert.deepEqual(band.rates.cache_write, { numerator: 0, denominator: 1 })
  }
  const refreshed = await mergeQuote('a/m', document, {
    ...quote,
    rates: { output: '2' },
  })
  assert.deepEqual(
    completeOfficialPrice(readOfficialToml('a/m', refreshed)),
    price
  )
})

test('missing cache prices stay unknown and tiered scalar placeholders cannot enable them', async () => {
  const rates = { input: '0.0000005', output: '1', request: '0' }
  const doc = await mergeQuote('a/m', writeOfficialToml('a/m'), {
    ...quote,
    rates,
  })
  const unchanged = readOfficialToml(
    'a/m',
    await mergeQuote('a/m', doc, { ...quote, rates })
  )
  const tier = unchanged.tiered
  assert.ok(tier)
  assert.equal(tier.uses_cache_read, false)
  assert.equal(tier.uses_cache_write, false)
  assert.equal(Object.hasOwn(tier.tiers[0].rates, 'cache_read'), false)
  assert.equal(Object.hasOwn(tier.tiers[0].rates, 'cache_write'), false)
  assert.throws(() => completeOfficialPrice(unchanged))
  const confirmed = readOfficialToml(
    'a/m',
    await mergeQuote('a/m', doc, {
      ...quote,
      rates: { cache_read: '0', cache_write: '0.0000005' },
    })
  )
  const complete = completeOfficialPrice(confirmed).tiered
  assert.ok(complete)
  assert.equal(complete.uses_cache_read, true)
  assert.equal(complete.uses_cache_write, true)
  assert.deepEqual(complete.tiers[0].rates.cache_read, {
    numerator: 0,
    denominator: 1,
  })
  assert.deepEqual(complete.tiers[0].rates.cache_write, {
    numerator: 1,
    denominator: 2000000,
  })
  // In a historic tiered tariff unused meters may carry a zero wire rate.
  const disabled = {
    ...confirmed,
    tiered: { ...complete, uses_cache_read: false },
  }
  const reload = await mergeQuote('a/m', writeOfficialToml('a/m', disabled), {
    ...quote,
    rates: {},
  })
  assert.equal(
    completeOfficialPrice(readOfficialToml('a/m', reload)).tiered
      ?.uses_cache_read,
    false
  )
})

test('publication requires one billing method and allows confirmed per-request or token rates', async () => {
  const requestOnly = {
    ...base,
    rates: { input: 0, output: 0, cache_read: 0, cache_write: 0, request: 25 },
  }
  assert.equal(completeOfficialPrice(requestOnly).rates.request, 25)
  assert.equal(completeOfficialPrice(base).rates.request, 0)
  for (const rates of [{ input: '1' }, { input: '0.0000005' }]) {
    const text = await mergeQuote(
      'a/m',
      writeOfficialToml('a/m', requestOnly),
      { ...quote, rates }
    )
    assert.throws(
      () => completeOfficialPrice(readOfficialToml('a/m', text)),
      /Set the request price to zero/
    )
  }
  const imported = await mergeQuote('a/m', writeOfficialToml('a/m', base), {
    ...quote,
    rates: { request: '2' },
  })
  assert.throws(
    () => completeOfficialPrice(readOfficialToml('a/m', imported)),
    /per-request billing/
  )
})

test('TTL preflight follows channel payloads and checks calendar tiers without guessing a provider', async () => {
  const {
    cacheWriteCoverageError: issue,
    officialPriceDestinations: destinations,
  } = await import('./official-cache-write-coverage')
  const routing = {
    upstreams: [
      { id: 1, protocol: 'openai_chat' },
      { id: 2, protocol: 'anthropic_messages' },
      { id: 3, protocol: 'gemini' },
    ],
    routes: [
      {
        model: 'neutral',
        speeds: ['standard'],
        targets: [
          {
            upstream_id: 1,
            payload: 'legacy_openrouter_chat_usage_v1',
            speeds: ['standard'],
          },
          { upstream_id: 3, payload: 'adapt', speeds: ['fast'] },
        ],
      },
    ],
  } as NativeConfiguration['routing']
  const legacy = destinations(routing, 'neutral', 1, 3)
  assert.deepEqual(legacy, [
    { protocol: 'openai_chat', payload: 'legacy_openrouter_chat_usage_v1' },
  ])
  assert.deepEqual(destinations(routing, 'neutral', undefined, 3), legacy)
  assert.deepEqual(destinations(routing, 'neutral', 2), [
    ...legacy,
    { protocol: 'anthropic_messages' },
  ])
  const unbound = 'openai-looking/name'
  const detached = destinations(routing, unbound)
  assert.deepEqual(detached, [])
  const openai = destinations(routing, unbound, undefined, 1)
  const anthropic = destinations(routing, unbound, 2)
  const gemini = destinations(routing, unbound, undefined, 3)
  const zero = { numerator: 0, denominator: 1 }
  const band = {
    rates: { input: zero, output: zero, cache_read: zero, cache_write: zero },
  }
  const price = {
    tiered: {
      revision: 2,
      source_digest: '0'.repeat(64),
      uses_cache_read: true,
      uses_cache_write: true,
      tiers: [band],
    },
  }
  assert.equal(issue(base, openai), undefined)
  assert.equal(issue(price, detached), undefined)
  assert.equal(issue(price, gemini), undefined)
  assert.equal(
    issue(
      { tiered: { ...price.tiered, revision: 1, uses_cache_write: false } },
      openai
    ),
    undefined
  )
  assert.match(
    issue({ tiered: { ...price.tiered, revision: 1 } }, openai) ?? '',
    /rates.cache_write_12h/
  )
  assert.match(issue(price, legacy) ?? '', /revision 3/)
  const hourBand = { rates: { ...band.rates, cache_write_1h: zero } }
  const hourly = {
    ...price.tiered,
    uses_cache_write_1h: true,
    tiers: [hourBand],
  }
  assert.equal(issue({ tiered: hourly }, anthropic), undefined)
  assert.match(
    issue(
      { tiered: { ...hourly, calendar: { match_tiers: [band] } } },
      anthropic
    ) ?? '',
    /rates.cache_write_1h/
  )
  assert.equal(issue({ tiered: { ...hourly, revision: 3 } }, legacy), undefined)
  assert.match(issue(price, [...openai, ...anthropic]) ?? '', /route targets/)
})

test('import diagnostics enumerate unquoted meters by band and preserve confirmed zero prices', async () => {
  const blank = readOfficialToml('a/m', writeOfficialToml('a/m'))
  assert.deepEqual(missingOfficialRatePaths(blank), [
    'rates.input',
    'rates.output',
    'rates.cache_read',
    'rates.cache_write',
    'rates.request',
  ])
  const quoteText = await mergeQuote('a/m', writeOfficialToml('a/m'), {
    ...quote,
    rates: { input: '1', output: '2', cache_read: '0' },
    context_tiers: [
      { above_input_tokens: 200000, rates: { cache_write: '3' } },
    ],
  })
  const draft = readOfficialToml('a/m', quoteText)
  const tariff = draft.tiered
  assert.ok(tariff)
  assert.deepEqual(missingOfficialRatePaths(draft, 'openai'), [
    'rates.request',
    'tiered.tiers[0].rates.cache_write',
    'tiered.tiers[0].rates.cache_write_12h',
    'tiered.tiers[1].rates.cache_write_12h',
  ])
  assert.throws(
    () =>
      completeOfficialPrice({
        ...draft,
        rates: { ...draft.rates, request: 0 },
      }),
    /tiered\.tiers\[0\]\.rates\.cache_write/
  )
  const zero = { numerator: 0, denominator: 1 }
  for (const band of tariff.tiers) {
    band.rates.cache_write ??= zero
    band.rates.cache_write_12h = zero
  }
  draft.rates.request = 0
  tariff.calendar = { match_tiers: [{ rates: { ...tariff.tiers[1].rates } }] }
  delete tariff.calendar.match_tiers[0].rates.cache_write_12h
  assert.deepEqual(missingOfficialRatePaths(draft, 'openai'), [
    'tiered.calendar.match_tiers[0].rates.cache_write_12h',
  ])
  tariff.calendar.match_tiers[0].rates.cache_write_12h = zero
  assert.deepEqual(missingOfficialRatePaths(draft, 'openai'), [])
  assert.deepEqual(missingOfficialRatePaths(draft, 'anthropic'), [
    'tiered.tiers[0].rates.cache_write_1h',
    'tiered.tiers[1].rates.cache_write_1h',
    'tiered.calendar.match_tiers[0].rates.cache_write_1h',
  ])
})
