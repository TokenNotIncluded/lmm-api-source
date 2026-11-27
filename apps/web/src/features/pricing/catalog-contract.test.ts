/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { adaptNativePricing } from './catalog-adapter'
import {
  nativePricingSchema,
  nativeImageVariantSchema,
  nativeRuntimeSchema,
  nativeUpstreamCostPriceSchema,
  type NativeCacheWrite12hTier,
  type NativePricingDocument,
} from './catalog-contract'
import { filterAndSortModels } from './lib/filters'
import {
  formatImageVariantCredits,
  formatImageVariantUSD,
} from './lib/image-price'
import { formatGroupPrice, formatPrice, formatRequestPrice } from './lib/price'
import { estimateRequestCost } from './lib/request-estimate'
import {
  getUpstreamCostPrice,
  formatUpstreamMultiplier,
} from './lib/upstream-cost'

const upstreamPrice = {
  currency: 'USD',
  cost_source: 'usage.cost',
  credits_per_usd: 500_000,
  rounding: 'ceil_total_credits',
  rate_basis: 'effective_group_rate',
  multiplier: { numerator: '1250000', denominator: '1000000' },
} as const

test('router prices preserve exact group factors and reject token estimates, including zero-token calls', () => {
  const input = fixture()
  const original = input.models[1]
  assert.ok(original)
  input.models[1] = {
    ...original,
    token_price: null,
    billing_status: 'unavailable',
    upstream_cost_price: upstreamPrice,
    unavailable_reason: 'upstream_budget_contract_required',
  }
  const [model] = adaptNativePricing(nativePricingSchema.parse(input)).data
  assert.ok(model)
  assert.equal(getUpstreamCostPrice(model, 'standard'), null)
  assert.equal(formatPrice(model, 'input', 'M', 'USD', 'standard'), '2 USD')
  assert.equal(formatPrice(model, 'input', 'M', 'USD', 'premium'), '-')
  const rule = getUpstreamCostPrice(model, 'premium')
  assert.ok(rule)
  assert.equal(formatUpstreamMultiplier(rule.price), '5/4')
  assert.equal(rule.reason, 'upstream_budget_contract_required')
  assert.equal(estimateRequestCost(model, 1, 0, 0, 0), null)
  const filters = {
    search: '',
    vendor: 'all',
    group: 'premium',
    quotaType: 'upstream_cost',
    endpointType: 'all',
    tag: 'all',
    sortBy: 'price-low',
  }
  const [routerGroup] = filterAndSortModels([model], filters)
  assert.ok(routerGroup)
  assert.equal(routerGroup.quota_type, 3)
  assert.equal(routerGroup.billing_mode, 'upstream_cost')
  assert.equal(formatPrice(routerGroup, 'input', 'M', 'USD'), '-')
  assert.equal(
    filterAndSortModels([model], { ...filters, quotaType: 'token' }).length,
    0
  )
  const [tokenGroup] = filterAndSortModels([model], {
    ...filters,
    group: 'standard',
    quotaType: 'token',
  })
  assert.ok(tokenGroup)
  assert.equal(tokenGroup.billing_mode, 'ratio')
  assert.equal(formatPrice(tokenGroup, 'input', 'M', 'USD'), '2 USD')
})

test('router multiplier boundary returns parse failures for malformed or unsafe fractions without throwing', () => {
  for (const multiplier of [
    { numerator: 'not-money', denominator: '1' },
    { numerator: '1', denominator: 'x' },
    { numerator: '1', denominator: '0' },
    { numerator: '0', denominator: '1' },
    { numerator: '1001', denominator: '1' },
  ]) {
    assert.equal(
      nativeUpstreamCostPriceSchema.safeParse({ ...upstreamPrice, multiplier })
        .success,
      false
    )
  }
  assert.equal(
    nativeUpstreamCostPriceSchema.safeParse(upstreamPrice).success,
    true
  )
  const input = fixture()
  const first = input.models[0]
  assert.ok(first)
  first.upstream_cost_price = upstreamPrice
  assert.equal(nativePricingSchema.safeParse(input).success, false)
})

test('router schema permits an independently priced image variant while retaining the text budget reason', () => {
  const input = fixture()
  const original = input.models[0]
  assert.ok(original)
  input.models[0] = {
    ...original,
    token_price: null,
    upstream_cost_price: upstreamPrice,
    unavailable_reason: 'upstream_budget_contract_required',
    image_variants: [
      {
        endpoint: 'generations',
        rate_basis: 'effective_group_rate',
        size: '1024x1024',
        quality: 'standard',
        max_n: 1,
        count_basis: 'requested',
        usd_micros_per_image: { numerator: '40000', denominator: '1' },
        credits_per_image: { numerator: '20000', denominator: '1' },
        reservation_credits_for_one: 20000,
        rounding: 'ceil_total_credits',
        pricing_version: 1,
      },
    ],
  }
  assert.equal(nativePricingSchema.safeParse(input).success, true)
  input.models[0].image_variants = []
  assert.equal(nativePricingSchema.safeParse(input).success, false)
})

function fixture(): NativePricingDocument {
  const token = {
    currency: 'USD',
    unit: 'million_tokens',
    rate_basis: 'effective_group_rate',
    group_multiplier: 5,
    input: 2,
    output: 6,
    cache_read: 0,
    cache_write: null,
    request: null,
    final_cost_depends_on_usage: true,
  } satisfies NonNullable<
    NativePricingDocument['models'][number]['token_price']
  >
  return {
    schema_version: 1,
    visibility: 'session',
    observed_at: 100,
    credits_per_usd: 500_000,
    rate_basis: 'effective_group_rate',
    invocation_authorized: false,
    routing_version: 2,
    pricing_version: 3,
    image_pricing_version: null,
    groups: ['standard', 'premium'],
    models: [
      {
        id: 'one',
        model: 'exact-model',
        group: 'standard',
        apis: ['openai-completions', 'system-one'],
        billing_status: 'configured',
        token_price: token,
        image_variants: [],
      },
      {
        id: 'two',
        model: 'exact-model',
        group: 'premium',
        apis: ['openai-completions'],
        billing_status: 'configured',
        token_price: { ...token, input: 4, output: 12 },
        image_variants: [],
      },
    ],
  }
}

test('empty metadata is bounded to the current view and public viewing is opt-in', () => {
  for (const visibility of ['session', 'public'] as const) {
    const empty = {
      ...fixture(),
      visibility,
      rate_basis:
        visibility === 'session' ? 'effective_group_rate' : 'base_group_rate',
      groups: [],
      models: [],
    }
    const reason =
      visibility === 'session' ? 'no_visible_models' : 'catalog_empty'
    const legacy = adaptNativePricing(nativePricingSchema.parse(empty))
    assert.equal(legacy.catalog_visibility, visibility)
    assert.equal(legacy.empty_reason, reason)
    assert.equal(legacy.public_view_available, false)
    const explicit = adaptNativePricing(
      nativePricingSchema.parse({
        ...empty,
        empty_reason: reason,
        public_view_available: true,
      })
    )
    assert.equal(explicit.empty_reason, reason)
    assert.equal(explicit.public_view_available, true)
    for (const empty_reason of [
      null,
      'hidden-private-model',
      visibility === 'session' ? 'catalog_empty' : 'no_visible_models',
    ]) {
      assert.equal(
        nativePricingSchema.safeParse({ ...empty, empty_reason }).success,
        false
      )
    }
  }
  assert.equal(
    nativePricingSchema.safeParse({
      ...fixture(),
      empty_reason: 'no_visible_models',
    }).success,
    false
  )
  assert.equal(
    nativePricingSchema.safeParse({
      ...fixture(),
      empty_reason: null,
      public_view_available: false,
    }).success,
    true
  )
})

test('native pricing parses the direct versioned document and keeps group rates separate', () => {
  const pricing = adaptNativePricing(nativePricingSchema.parse(fixture()))
  assert.equal(pricing.data.length, 1)
  const model = pricing.data[0]
  assert.ok(model)
  assert.equal(model.input_price, null)
  assert.equal(model.cache_read_price, 0)
  assert.equal(formatPrice(model, 'input', 'M', 'USD'), '-')
  assert.equal(formatPrice(model, 'input', 'M', 'USD', 'standard'), '2 USD')
  assert.equal(
    formatGroupPrice(model, 'premium', 'input', 'M', 'USD', { premium: 100 }),
    '4 USD'
  )
  assert.deepEqual(model.supported_endpoint_types, ['openai', 'systemone'])
  assert.equal(pricing.supported_endpoint.systemone?.path, '/v1/systemone')
  assert.equal(model.runtime_state?.status, 'unknown')
  assert.equal(model.performance_available, false)
})

test('public pricing exposes configured group rates without account pricing metadata', () => {
  const input = fixture()
  input.visibility = 'public'
  input.rate_basis = 'base_group_rate'
  for (const row of input.models) {
    if (row.token_price) row.token_price.rate_basis = 'base_group_rate'
  }
  const pricing = adaptNativePricing(nativePricingSchema.parse(input))
  assert.deepEqual(pricing.pricing_context, { rate_basis: 'base_group_rate' })
  assert.deepEqual(pricing.data[0]?.pricing_context, pricing.pricing_context)
  const model = pricing.data[0]
  assert.ok(model)
  assert.equal(formatPrice(model, 'input', 'M', 'USD', 'standard'), '2 USD')
})

test('session prices use returned group rates without a second multiplication', () => {
  const input = fixture()
  for (const row of input.models) {
    if (row.token_price) {
      row.token_price.input = 1.94
      row.token_price.output = 0
      row.token_price.cache_read = null
    }
  }
  const pricing = adaptNativePricing(nativePricingSchema.parse(input))
  const model = pricing.data[0]
  assert.ok(model)
  assert.deepEqual(model.pricing_context, {
    rate_basis: 'effective_group_rate',
  })
  assert.equal(formatPrice(model, 'input', 'M', 'USD', 'standard'), '1.94 USD')
  assert.equal(
    formatGroupPrice(model, 'premium', 'input', 'M', 'USD', { premium: 500 }),
    '1.94 USD'
  )
  assert.equal(formatPrice(model, 'output', 'M', 'USD', 'standard'), '0 USD')
  assert.equal(formatPrice(model, 'cache', 'M', 'USD', 'standard'), '-')
})

test('public and session price bases stay distinct without trust metadata', () => {
  const session = fixture()
  const publicDocument = {
    ...session,
    visibility: 'public',
    rate_basis: 'base_group_rate',
    models: [],
  }
  assert.equal(nativePricingSchema.safeParse(publicDocument).success, true)
  assert.equal(nativePricingSchema.safeParse(session).success, true)
  for (const bad of [
    { ...session, visibility: 'public' },
    { ...publicDocument, visibility: 'session' },
    { ...publicDocument, models: session.models },
    { ...session, rate_basis: 'effective_user_rate' },
  ]) {
    assert.equal(nativePricingSchema.safeParse(bad).success, false)
  }
  const withRetiredFields = nativePricingSchema.parse({
    ...session,
    user_price: { status: 'not_evaluated' },
    trust_multiplier: 0,
  })
  assert.equal(Object.hasOwn(withRetiredFields, 'user_price'), false)
  assert.deepEqual(adaptNativePricing(withRetiredFields).pricing_context, {
    rate_basis: 'effective_group_rate',
  })
})

test('native image rates use the same configured group basis as the session document', () => {
  const input = fixture()
  const original = input.models[0]
  assert.ok(original)
  input.models = [
    {
      ...original,
      token_price: null,
      image_variants: [
        {
          endpoint: 'generations',
          rate_basis: 'effective_group_rate',
          size: '1024x1024',
          quality: 'standard',
          max_n: 1,
          count_basis: 'requested',
          usd_micros_per_image: { numerator: '40000', denominator: '1' },
          credits_per_image: { numerator: '20000', denominator: '1' },
          reservation_credits_for_one: 20000,
          rounding: 'ceil_total_credits',
          pricing_version: 3,
        },
      ],
    },
  ]
  const model = adaptNativePricing(nativePricingSchema.parse(input)).data[0]
  assert.ok(model)
  const image = model.image_variants?.[0]
  assert.ok(image)
  assert.equal(image.rate_basis, 'effective_group_rate')
  assert.equal(formatImageVariantUSD(image), '0.04 USD')
  assert.equal(formatImageVariantCredits(image), '20000 Credits')
  const sourceVariant = input.models[0]?.image_variants[0]
  assert.ok(sourceVariant)
  sourceVariant.rate_basis = 'base_group_rate'
  assert.equal(nativePricingSchema.safeParse(input).success, false)
  delete sourceVariant.rate_basis
  assert.equal(nativePricingSchema.safeParse(input).success, false)
})

test('selected group drives sorting and never substitutes unknown prices with zero', () => {
  const input = fixture()
  const original = input.models[0]
  assert.ok(original)
  input.models.push({
    ...original,
    id: 'three',
    model: 'unknown',
    token_price: null,
    billing_status: 'unavailable',
  })
  const models = adaptNativePricing(nativePricingSchema.parse(input)).data
  const filtered = filterAndSortModels(models, {
    search: '',
    vendor: 'all',
    group: 'standard',
    quotaType: 'all',
    endpointType: 'all',
    tag: 'all',
    sortBy: 'price-low',
  })
  assert.equal(filtered[0]?.model_name, 'exact-model')
  assert.equal(filtered[0]?.input_price, 2)
  assert.equal(filtered[1]?.model_name, 'unknown')
})

test('token null remains unknown while an explicit zero remains free', () => {
  const input = fixture()
  const token = input.models[0]?.token_price
  const original = input.models[0]
  assert.ok(token)
  assert.ok(original)
  input.models = [
    { ...original, token_price: { ...token, input: null, output: 0 } },
  ]
  const model = adaptNativePricing(nativePricingSchema.parse(input)).data[0]
  assert.ok(model)
  assert.equal(formatPrice(model, 'input', 'M', 'USD'), '-')
  assert.equal(formatPrice(model, 'output', 'M', 'USD'), '0 USD')
})

test('group projection cannot borrow another group protocol or token prices', () => {
  const input = fixture()
  const second = input.models[1]
  assert.ok(second)
  second.token_price = null
  second.apis = ['openai-images-generations']
  const pricing = adaptNativePricing(nativePricingSchema.parse(input))
  const model = pricing.data[0]
  assert.ok(model)
  assert.equal(model.input_price, null)
  assert.equal(formatPrice(model, 'input', 'M', 'USD', 'premium'), '-')
  const filters = {
    search: '',
    vendor: 'all',
    group: 'premium',
    quotaType: 'all',
    endpointType: 'all',
    tag: 'all',
    sortBy: 'name',
  }
  const selected = filterAndSortModels(pricing.data, filters)
  assert.equal(selected[0]?.input_price, null)
  assert.equal(selected[0]?.quota_type, 2)
  assert.deepEqual(selected[0]?.supported_endpoint_types, ['image-generation'])
  assert.equal(
    filterAndSortModels(pricing.data, { ...filters, endpointType: 'systemone' })
      .length,
    0
  )
  assert.equal(
    filterAndSortModels(pricing.data, { ...filters, search: 'systemone' })
      .length,
    0
  )
  assert.equal(
    filterAndSortModels(pricing.data, { ...filters, quotaType: 'token' })
      .length,
    0
  )
})

test('image exact ratios stay per-image and cannot become token quotes', () => {
  const input = fixture()
  const variant = {
    endpoint: 'generations',
    rate_basis: 'effective_group_rate',
    size: '1024x1024',
    quality: 'standard',
    max_n: 4,
    count_basis: 'returned_payloads',
    usd_micros_per_image: { numerator: '80000', denominator: '2' },
    credits_per_image: { numerator: '20000', denominator: '1' },
    reservation_credits_for_one: 20000,
    rounding: 'ceil_total_credits',
    pricing_version: 3,
  } satisfies NonNullable<
    NativePricingDocument['models'][number]
  >['image_variants'][number]
  input.models = [
    {
      id: 'image',
      model: 'image-model',
      group: 'standard',
      apis: ['openai-images-generations'],
      billing_status: 'configured',
      token_price: null,
      image_variants: [variant],
    },
  ]
  const model = adaptNativePricing(nativePricingSchema.parse(input)).data[0]
  assert.ok(model)
  const image = model.image_variants?.[0]
  assert.ok(image)
  assert.equal(model.quota_type, 2)
  assert.equal(model.input_price, null)
  assert.equal(model.image_price, undefined)
  assert.equal(formatImageVariantUSD(image), '0.04 USD')
  assert.equal(formatImageVariantCredits(image), '20000 Credits')
  variant.credits_per_image.numerator = '20001'
  assert.equal(nativePricingSchema.safeParse(input).success, false)
})

test('invalid denomination, unit, duplicate group model and legacy envelopes fail at the boundary', () => {
  const input = fixture()
  for (const bad of [
    { success: true, data: [] },
    { ...input, credits_per_usd: 1 },
    { ...input, invocation_authorized: true },
    { ...input, models: [...input.models, input.models[0]] },
    {
      ...input,
      models: [
        {
          ...input.models[0],
          token_price: { ...input.models[0]?.token_price, unit: 'image' },
        },
      ],
    },
  ]) {
    assert.equal(nativePricingSchema.safeParse(bad).success, false)
  }
})

test('native tools retain successful request units, group prices and exact endpoint paths', () => {
  const input = fixture()
  const first = input.models[0]
  assert.ok(first?.token_price)
  input.models = [
    {
      ...first,
      model: 'deepseek-search',
      apis: ['deepseek-search'],
      token_price: {
        ...first.token_price,
        unit: 'successful_requests',
        input: null,
        output: null,
        cache_read: null,
        request: 0.00075,
      },
    },
    {
      ...first,
      id: 'fetch',
      model: 'deepseek-fetch',
      apis: ['deepseek-fetch'],
      token_price: {
        ...first.token_price,
        unit: 'successful_requests',
        input: null,
        output: null,
        cache_read: null,
        request: 0,
      },
    },
  ]
  const output = adaptNativePricing(nativePricingSchema.parse(input))
  const [search, fetch] = output.data
  assert.ok(search && fetch)
  assert.equal(search.quota_type, 1)
  assert.equal(search.group_quota_types?.standard, 1)
  assert.equal(search.input_price, null)
  assert.equal(search.model_price, 0.00075)
  assert.equal(search.group_prices?.standard?.model_price, 0.00075)
  assert.equal(formatPrice(search, 'input', 'M', 'USD'), '-')
  assert.notEqual(formatRequestPrice(search, 'USD', 'standard'), '-')
  assert.equal(fetch.model_price, 0)
  assert.equal(
    output.supported_endpoint['deepseek-search']?.path,
    '/v1/tools/search'
  )
  assert.equal(
    output.supported_endpoint['deepseek-fetch']?.path,
    '/v1/tools/fetch'
  )
  const price = input.models[0]?.token_price
  assert.ok(price)
  price.input = 1
  assert.equal(nativePricingSchema.safeParse(input).success, false)
  price.input = null
  price.unit = 'million_tokens'
  assert.equal(nativePricingSchema.safeParse(input).success, false)
})

test('native input APIs map to their own request endpoints', () => {
  const input = fixture()
  const first = input.models[0]
  assert.ok(first)
  input.models = [
    { ...first, model: 'embed', apis: ['openai-embeddings'] },
    { ...first, id: 'rerank', model: 'rerank', apis: ['jina-rerank'] },
  ]
  const output = adaptNativePricing(nativePricingSchema.parse(input))
  assert.equal(output.supported_endpoint.embeddings?.path, '/v1/embeddings')
  assert.equal(output.supported_endpoint['jina-rerank']?.path, '/v1/rerank')
  assert.deepEqual(output.data[1]?.supported_endpoint_types, ['jina-rerank'])
})

test('runtime projection cannot turn configuration into a live healthy badge', () => {
  const document = {
    schema_version: 1,
    visibility: 'public',
    observed_at: 100,
    data: {
      model: {
        status: 'unknown',
        source: 'routing_configuration',
        observed_at: 100,
      },
    },
  }
  assert.equal(nativeRuntimeSchema.safeParse(document).success, true)
  assert.equal(
    nativeRuntimeSchema.safeParse({
      ...document,
      data: { model: { ...document.data.model, status: 'available' } },
    }).success,
    false
  )
})

test('12-hour cache pricing keeps each group and tier without a fictional scalar price', () => {
  const document = fixture()
  const fraction = (numerator: string, denominator = '1') => ({
    numerator,
    denominator,
  })
  const tier: NativeCacheWrite12hTier = {
    up_to_input_tokens: '272000',
    input: fraction('10'),
    output: fraction('50'),
    cache_read: fraction('1'),
    cache_write: fraction('25', '2'),
    cache_write_12h: fraction('25'),
  }
  for (const [index, row] of document.models.entries()) {
    assert.ok(row.token_price)
    row.billing_status = 'usage_dependent'
    row.token_price = {
      ...row.token_price,
      input: null,
      output: null,
      cache_read: null,
      cache_write_12h_tiers: [
        tier,
        {
          ...tier,
          up_to_input_tokens: null,
          cache_write_12h: fraction(index === 0 ? '50' : '0'),
        },
      ],
    }
  }
  const parsed = nativePricingSchema.parse(document)
  const [model] = adaptNativePricing(parsed).data
  assert.ok(model)
  assert.equal(model.input_price, null)
  assert.equal(model.cache_write_price, null)
  assert.equal(
    model.group_cache_write_12h_tiers?.standard?.[0]?.up_to_input_tokens,
    '272000'
  )
  assert.equal(
    model.group_cache_write_12h_tiers?.standard?.[1]?.cache_write_12h.numerator,
    '50'
  )
  assert.equal(
    model.group_cache_write_12h_tiers?.premium?.[1]?.cache_write_12h.numerator,
    '0'
  )
  for (const mutate of [
    (tiers: NativeCacheWrite12hTier[]) => {
      tiers[0]!.cache_write_12h = fraction('-1')
    },
    (tiers: NativeCacheWrite12hTier[]) => {
      tiers[0]!.cache_write_12h = fraction('1', '0')
    },
    (tiers: NativeCacheWrite12hTier[]) => {
      tiers[0]!.up_to_input_tokens = '18446744073709551616'
    },
    (tiers: NativeCacheWrite12hTier[]) => {
      tiers[1]!.up_to_input_tokens = '1'
    },
    (tiers: NativeCacheWrite12hTier[]) => {
      tiers[0]!.up_to_input_tokens = 'invalid'
    },
    (tiers: NativeCacheWrite12hTier[]) => {
      tiers.push(...tiers, ...tiers)
    },
    (tiers: NativeCacheWrite12hTier[]) => {
      tiers[0]!.up_to_input_tokens = null
    },
    (tiers: NativeCacheWrite12hTier[]) => {
      tiers.splice(1, 0, { ...tiers[0]!, up_to_input_tokens: '272000' })
    },
    (tiers: NativeCacheWrite12hTier[]) => {
      tiers.splice(1, 0, { ...tiers[0]!, up_to_input_tokens: '1' })
    },
    (tiers: NativeCacheWrite12hTier[]) => {
      tiers.length = 0
    },
  ]) {
    const invalid = structuredClone(document)
    mutate(invalid.models[0]!.token_price!.cache_write_12h_tiers!)
    assert.equal(nativePricingSchema.safeParse(invalid).success, false)
  }
  const wrongBasis = structuredClone(document)
  wrongBasis.models[0]!.token_price!.rate_basis = 'base_group_rate'
  assert.equal(nativePricingSchema.safeParse(wrongBasis).success, false)
  const scalar = structuredClone(document)
  scalar.models[0]!.token_price!.input = 10
  assert.equal(nativePricingSchema.safeParse(scalar).success, false)
  assert.equal(
    nativePricingSchema.parse(fixture()).models[0]?.token_price
      ?.cache_write_12h_tiers,
    undefined
  )
})

test('usage-priced image variants preserve null prices and operation reservation', () => {
  const variant = {
    endpoint: 'generations',
    rate_basis: 'effective_group_rate',
    size: '1024x1024',
    quality: 'standard',
    max_n: 4,
    count_basis: {
      native_tokens: {
        source_expression: 'tier("standard", p * 5 + img_o * 30)',
        billing_price_key: 'native-image',
        source_sha256: 'a'.repeat(64),
        currency_basis: 'legacy_pricing_unit',
        credits_per_usd: 500000,
        usd_multiplier: { numerator: 1, denominator: 1 },
        max_input_tokens: 4096,
        max_output_tokens: 8192,
      },
    },
    usd_micros_per_image: null,
    credits_per_image: null,
    reservation_credits_for_one: 140000,
    reservation_scope: 'per_operation',
    rounding: 'half_up_total_credits_positive_minimum',
    pricing_version: 4,
  }
  const input = fixture()
  input.models = [
    {
      id: 'native-image',
      model: 'native-image',
      group: 'standard',
      apis: ['openai-images-generations'],
      billing_status: 'configured',
      token_price: null,
      image_variants: [nativeImageVariantSchema.parse(variant)],
    },
  ]
  const model = adaptNativePricing(nativePricingSchema.parse(input)).data[0]
  const image = model.image_variants?.[0]
  assert.ok(image)
  assert.equal(formatImageVariantUSD(image), null)
  assert.equal(formatImageVariantCredits(image), null)
  assert.equal(image.reservation_scope, 'per_operation')
  assert.equal(image.reservation_credits_for_one, 140000)
  const ratioVariant = nativeImageVariantSchema.parse({
    ...variant,
    count_basis: {
      legacy_token_ratio: {
        billing_price_key: 'image-ratio',
        model_ratio: { numerator: 1, denominator: 4 },
        completion_ratio: { numerator: 6, denominator: 1 },
        cache_ratio: { numerator: 1, denominator: 10 },
        image_ratio: { numerator: 1, denominator: 1 },
        legacy_quota_per_unit: 500000,
        credits_per_usd: 500000,
        usd_multiplier: { numerator: 1, denominator: 1 },
        max_input_tokens: 4096,
        max_output_tokens: 8192,
      },
    },
  })
  assert.equal(ratioVariant.credits_per_image, null)
  assert.equal(ratioVariant.reservation_scope, 'per_operation')
  for (const wrong of [
    { ...variant, reservation_scope: 'per_image' },
    { ...variant, reservation_scope: undefined },
    { ...variant, credits_per_image: { numerator: '0', denominator: '1' } },
    { ...variant, rounding: 'ceil_total_credits' },
    { ...variant, count_basis: 'requested' },
  ]) {
    assert.equal(nativeImageVariantSchema.safeParse(wrong).success, false)
  }
})

test('catalog metadata preserves per-group original ids, unknown context and effective rates', () => {
  const input = fixture()
  const original = input.models[0]
  assert.ok(original)
  input.models[0] = {
    ...original,
    upstream_model: 'provider-original',
    metadata: {
      context_window: null,
      max_input_tokens: 131072,
      max_output_tokens: 8192,
      input_modalities: null,
      output_modalities: null,
      reasoning: null,
      tool_call: null,
      source: 'reviewed_provider_contract',
    },
  }
  const model = adaptNativePricing(nativePricingSchema.parse(input)).data[0]
  assert.ok(model)
  assert.deepEqual(model.catalog_metadata?.standard, {
    original_id: 'provider-original',
    context_window: null,
    max_input_tokens: 131072,
    max_output_tokens: 8192,
    group_multiplier: input.models[0].token_price?.group_multiplier,
  })
  assert.equal(model.catalog_metadata?.premium.context_window, null)
  assert.equal(model.catalog_metadata?.premium.original_id, undefined)
  assert.equal(formatPrice(model, 'input', 'M', 'USD', 'standard'), '2 USD')
})

test('pricing defaults fill the vendor directory from published model IDs', () => {
  const input = fixture()
  const base = input.models[0]
  input.models = [
    { ...base, model: 'claude-haiku-4-5-20251001' },
    { ...base, model: 'google/gemma-4-31b-it:free' },
    { ...base, model: 'recursive/flint-v1.0:free' },
  ]
  const pricing = adaptNativePricing(nativePricingSchema.parse(input))
  const names = new Map(pricing.vendors.map(({ id, name }) => [id, name]))
  assert.deepEqual(
    pricing.data.map((model) => [names.get(model.vendor_id ?? 0), model.icon]),
    [
      ['Anthropic', 'Claude.Color'],
      ['Google', 'Gemini.Color'],
      ['Recursive', undefined],
    ]
  )
  assert.equal(pricing.performance_available, false)
})

test('constant tariffs expose the effective group quote and keep missing meters separate from a zero price', () => {
  const doc = fixture()
  const row = doc.models[0]
  const token = row.token_price!
  const zero = { numerator: '0', denominator: '1' }
  const constants = {
    input: { numerator: '1', denominator: '2000000' },
    output: { numerator: '3', denominator: '2' },
    cache_read: zero,
    cache_write: null,
  }
  row.token_price = {
    ...token,
    input: null,
    output: null,
    cache_read: null,
    constant_token_rates: constants,
  }
  const model = adaptNativePricing(nativePricingSchema.parse(doc)).data[0]
  assert.equal(model.group_prices?.standard.input_price, 0.0000005)
  assert.equal(model.group_prices?.standard.output_price, 1.5)
  assert.equal(model.group_prices?.standard.cache_read_price, 0)
  assert.equal(model.group_prices?.standard.cache_write_price, null)
  assert.equal(model.input_price, null)
  assert.equal(
    nativePricingSchema.safeParse({
      ...doc,
      models: [{ ...row, token_price: { ...row.token_price, input: 0 } }],
    }).success,
    false
  )
  assert.equal(
    nativePricingSchema.safeParse({
      ...doc,
      models: [
        {
          ...row,
          token_price: {
            ...row.token_price,
            constant_token_rates: {
              ...constants,
              input: { numerator: '1', denominator: '0' },
            },
          },
        },
      ],
    }).success,
    false
  )
})
