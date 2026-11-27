/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import { resolveModelProvider } from '@/lib/model-provider'

import type { NativePricingDocument } from './catalog-contract'
import type {
  PricingData,
  PricingGroupPrices,
  PricingModel,
  PricingVendor,
} from './types'

const endpoints: Record<
  string,
  { type: string; path?: string; method?: string }
> = {
  'openai-completions': {
    type: 'openai',
    path: '/v1/chat/completions',
    method: 'POST',
  },
  'openai-responses': {
    type: 'openai-response',
    path: '/v1/responses',
    method: 'POST',
  },
  'anthropic-messages': {
    type: 'anthropic',
    path: '/v1/messages',
    method: 'POST',
  },
  'google-generative-ai': {
    type: 'gemini',
    path: '/v1beta/models/{model}:generateContent',
    method: 'POST',
  },
  'system-one': { type: 'systemone', path: '/v1/systemone', method: 'POST' },
  'openai-embeddings': {
    type: 'embeddings',
    path: '/v1/embeddings',
    method: 'POST',
  },
  'jina-rerank': { type: 'jina-rerank', path: '/v1/rerank', method: 'POST' },
  'deepseek-search': {
    type: 'deepseek-search',
    path: '/v1/tools/search',
    method: 'POST',
  },
  'deepseek-fetch': {
    type: 'deepseek-fetch',
    path: '/v1/tools/fetch',
    method: 'POST',
  },
  'openai-images-generations': {
    type: 'image-generation',
    path: '/v1/images/generations',
    method: 'POST',
  },
  // Image edits use multipart input; do not copy a JSON generation example.
  'openai-images-edits': { type: 'image-edits' },
}

function groupPrices(
  row: NativePricingDocument['models'][number]
): PricingGroupPrices {
  const token = row.billing_status === 'configured' ? row.token_price : null
  // A lifetime-dependent write price can still have fixed input, output and read rates.
  const constant =
    row.billing_status === 'unavailable'
      ? undefined
      : row.token_price?.constant_token_rates
  // Keep numeric projections for sorting and exchange-rate conversions.
  const amount = (rate: { numerator: string; denominator: string } | null) =>
    rate ? Number(rate.numerator) / Number(rate.denominator) : null
  return {
    input_price: constant ? amount(constant.input) : (token?.input ?? null),
    output_price: constant ? amount(constant.output) : (token?.output ?? null),
    cache_read_price: constant
      ? amount(constant.cache_read)
      : (token?.cache_read ?? null),
    cache_write_price: constant
      ? amount(constant.cache_write)
      : (token?.cache_write ?? null),
    model_price: token?.unit === 'successful_requests' ? token.request : null,
    image_price: null,
    audio_input_price: null,
    audio_output_price: null,
  }
}

function quotaType(
  row: NativePricingDocument['models'][number]
): 0 | 1 | 2 | 3 {
  if (row.token_price?.unit === 'successful_requests') return 1
  return row.token_price ? 0 : row.upstream_cost_price ? 3 : 2
}

/** A shared headline rate exists only when every visible group agrees. */
function commonPrice(
  prices: PricingGroupPrices[],
  field: keyof PricingGroupPrices
): number | null {
  const value = prices[0]?.[field] ?? null
  return prices.every((price) => price[field] === value) ? value : null
}

export function adaptNativePricing(
  document: NativePricingDocument
): PricingData {
  const pricingContext = {
    rate_basis: document.rate_basis,
  }
  const grouped = new Map<string, NativePricingDocument['models']>()
  for (const row of document.models) {
    const rows = grouped.get(row.model) ?? []
    rows.push(row)
    grouped.set(row.model, rows)
  }
  const models: PricingModel[] = []
  const vendors = new Map<string, PricingVendor>()
  const endpointMap: PricingData['supported_endpoint'] = {}
  for (const [name, rows] of grouped) {
    const tokenRows = rows.filter(
      (row) => row.token_price?.unit === 'million_tokens'
    )
    const requestRows = rows.filter(
      (row) => row.token_price?.unit === 'successful_requests'
    )
    const upstreamCostRows = rows.filter((row) => row.upstream_cost_price)
    const prices = rows.map(groupPrices)
    const apis = new Set<string>()
    for (const row of rows) {
      for (const api of row.apis) {
        const endpoint = endpoints[api]
        if (!endpoint) continue
        apis.add(endpoint.type)
        if (endpoint.path && endpoint.method) {
          endpointMap[endpoint.type] = {
            path: endpoint.path,
            method: endpoint.method,
          }
        }
      }
    }
    // Prefer the published model family. A custom alias can use a vendor
    // inferred from its upstream IDs only when every visible group agrees.
    let provider = resolveModelProvider(name)
    if (!provider) {
      const upstreamProviders = rows.map((row) =>
        row.upstream_model ? resolveModelProvider(row.upstream_model) : null
      )
      const first = upstreamProviders[0]
      if (
        first &&
        upstreamProviders.every((item) => item?.label === first.label)
      ) {
        provider = first
      }
    }
    let vendor = provider ? vendors.get(provider.label) : undefined
    if (provider && !vendor) {
      vendor = {
        id: vendors.size + 1,
        name: provider.label,
        icon: provider.icon,
      }
      vendors.set(provider.label, vendor)
    }
    models.push({
      pricing_context: pricingContext,
      id: models.length + 1,
      key: name,
      model_name: name,
      vendor_id: vendor?.id,
      icon: vendor?.icon,
      catalog_metadata: Object.fromEntries(
        rows.map((row) => [
          row.group,
          {
            original_id: row.upstream_model,
            context_window: row.metadata?.context_window ?? null,
            max_input_tokens: row.metadata?.max_input_tokens ?? null,
            max_output_tokens: row.metadata?.max_output_tokens ?? null,
            group_multiplier: row.token_price?.group_multiplier ?? null,
          },
        ])
      ),
      pricing_schema_version: 2,
      pricing_currency: 'USD',
      pricing_version: String(document.pricing_version),
      quota_type: tokenRows.length
        ? 0
        : requestRows.length
          ? 1
          : upstreamCostRows.length
            ? 3
            : 2,
      billing_mode:
        upstreamCostRows.length === rows.length ? 'upstream_cost' : 'ratio',
      group_upstream_cost_prices: Object.fromEntries(
        upstreamCostRows.flatMap((row) =>
          row.upstream_cost_price ? [[row.group, row.upstream_cost_price]] : []
        )
      ),
      group_unavailable_reasons: Object.fromEntries(
        rows.flatMap((row) =>
          row.unavailable_reason ? [[row.group, row.unavailable_reason]] : []
        )
      ),
      input_price: commonPrice(prices, 'input_price'),
      output_price: commonPrice(prices, 'output_price'),
      cache_read_price: commonPrice(prices, 'cache_read_price'),
      cache_write_price: commonPrice(prices, 'cache_write_price'),
      model_price: commonPrice(prices, 'model_price') ?? undefined,
      group_constant_token_rates: Object.fromEntries(
        rows.flatMap((row) =>
          row.billing_status !== 'unavailable' &&
          row.token_price?.constant_token_rates
            ? [[row.group, row.token_price.constant_token_rates]]
            : []
        )
      ),
      group_cache_write_12h_tiers: Object.fromEntries(
        rows.flatMap((row) =>
          row.token_price?.cache_write_12h_tiers
            ? [[row.group, row.token_price.cache_write_12h_tiers]]
            : []
        )
      ),
      group_prices:
        tokenRows.length || requestRows.length
          ? Object.fromEntries(rows.map((row) => [row.group, groupPrices(row)]))
          : undefined,
      group_endpoints: Object.fromEntries(
        rows.map((row) => [
          row.group,
          row.apis.flatMap((api) => endpoints[api]?.type ?? []),
        ])
      ),
      group_quota_types: Object.fromEntries(
        rows.map((row) => [row.group, quotaType(row)])
      ),
      image_variants: rows.flatMap((row) =>
        row.image_variants.map((variant) => ({
          ...variant,
          group: row.group,
          model: row.model,
        }))
      ),
      enable_groups: rows.map((row) => row.group),
      supported_endpoint_types: [...apis],
      group_ratio: Object.fromEntries(rows.map((row) => [row.group, 1])),
      runtime_state: {
        status: rows.every((row) => row.billing_status === 'unavailable')
          ? 'unavailable'
          : 'unknown',
        source: 'routing_configuration',
        observed_at: document.observed_at,
      },
      performance_available: false,
    })
  }
  return {
    catalog_visibility: document.visibility,
    empty_reason:
      document.empty_reason ??
      (document.models.length === 0
        ? document.visibility === 'session'
          ? 'no_visible_models'
          : 'catalog_empty'
        : null),
    public_view_available: document.public_view_available === true,
    pricing_context: pricingContext,
    success: true,
    pricing_schema_version: 2,
    pricing_currency: 'USD',
    credits_per_usd: document.credits_per_usd,
    data: models,
    vendors: [...vendors.values()].sort((left, right) =>
      left.name.localeCompare(right.name)
    ),
    group_ratio: Object.fromEntries(document.groups.map((group) => [group, 1])),
    usable_group: Object.fromEntries(
      document.groups.map((group) => [group, { desc: group, ratio: 1 }])
    ),
    auto_groups: [],
    supported_endpoint: endpointMap,
    runtime_available: true,
    performance_available: false,
  }
}
