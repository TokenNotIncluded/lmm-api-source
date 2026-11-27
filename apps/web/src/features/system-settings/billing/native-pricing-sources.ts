/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { isLosslessNumber } from 'lossless-json'

import { usdToMicros } from '@/features/channels/components/native-model-draft'

import {
  decimalPriceFraction,
  type NativePriceRule,
} from './native-price-rules'
import { fetchModelsDevCatalog } from './official-models-dev-quote'

export type CatalogRates = Partial<NativePriceRule['rates']>
export type ModelCatalogPrice = {
  provider: string
  name: string
  rates: CatalogRates
}

const catalogFields = ['input', 'output', 'cache_read', 'cache_write'] as const

function record(value: unknown): Record<string, unknown> | undefined {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return
  return value as Record<string, unknown>
}

export function modelsDevPrices(
  document: unknown,
  model: string
): ModelCatalogPrice[] {
  if (!model || model.length > 200) return []
  return Object.entries(record(document) ?? {})
    .flatMap(([provider, raw]) => {
      const source = record(raw)
      const models = record(source?.models)
      if (!models || !Object.hasOwn(models, model)) return []
      const costs = record(record(models[model])?.cost)
      if (!costs) return []
      const rates: CatalogRates = {}
      for (const key of catalogFields) {
        if (!Object.hasOwn(costs, key) && key !== 'input' && key !== 'output') {
          continue
        }
        const value = costs[key]
        if (!isLosslessNumber(value)) return []
        const micros = usdToMicros(value.value)
        if (micros === null) return []
        rates[key] = micros
      }
      return [
        {
          provider,
          name: typeof source?.name === 'string' ? source.name : provider,
          rates,
        },
      ]
    })
    .sort(
      (a, b) =>
        a.name.localeCompare(b.name) || a.provider.localeCompare(b.provider)
    )
}

export async function fetchModelsDevPrices(model: string, signal: AbortSignal) {
  return modelsDevPrices(await fetchModelsDevCatalog(signal), model)
}

const coefficients = {
  p: 'input',
  c: 'output',
  cr: 'cache_read',
  cc: 'cache_write',
} as const

export function compileLinearExpression(expression: string) {
  if (expression.length > 1024) return null
  const formula = /^tier\("[A-Za-z0-9_-]{1,40}",\s*([^()]+)\)$/.exec(
    expression.trim()
  )
  if (!formula) return null
  const seen = new Set<string>()
  const zero: NonNullable<ReturnType<typeof decimalPriceFraction>> = {
    numerator: 0,
    denominator: 1,
  }
  const rates = {
    input: zero,
    output: zero,
    cache_read: zero,
    cache_write: zero,
  }
  for (const term of formula[1].split('+')) {
    const match = /^(p|c|cr|cc)\s*\*\s*(\d+(?:\.\d{1,18})?)$/.exec(term.trim())
    if (!match || seen.has(match[1])) return null
    const variable = match[1] as keyof typeof coefficients
    const rate = decimalPriceFraction(match[2])
    if (!rate) return null
    seen.add(variable)
    rates[coefficients[variable]] = rate
  }
  if (!seen.has('p') || !seen.has('c')) return null
  return {
    revision: 2,
    uses_cache_read: seen.has('cr'),
    uses_cache_write: seen.has('cc'),
    tiers: [{ up_to_input_tokens: null, rates }],
  }
}

export async function expressionPriceRule(
  rule: NativePriceRule,
  expression: string
) {
  const tariff = compileLinearExpression(expression)
  if (
    !tariff ||
    rule.rates.request !== 0 ||
    rule.billing_mode === 'upstream_cost'
  ) {
    return null
  }
  const digest = await globalThis.crypto.subtle.digest(
    'SHA-256',
    new TextEncoder().encode(expression.trim())
  )
  const sourceDigest = Array.from(new Uint8Array(digest), (byte) =>
    byte.toString(16).padStart(2, '0')
  ).join('')
  // The expression replaces the prior token tariff, while keeping selector and
  // multipliers. Core's versioned preview still validates the resulting draft.
  const next: NativePriceRule = {
    ...rule,
    billing_mode: 'tiered_decimal',
    rates: { input: 0, output: 0, cache_read: 0, cache_write: 0, request: 0 },
    tiered: { ...tariff, source_digest: sourceDigest },
  }
  delete next.rounding
  return next
}
