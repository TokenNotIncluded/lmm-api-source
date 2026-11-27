/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { isLosslessNumber, parse } from 'lossless-json'

const catalogURL = 'https://models.dev/api.json'
const catalogByteLimit = 12 * 1024 * 1024

export async function fetchModelsDevCatalog(
  signal?: AbortSignal
): Promise<unknown> {
  const timeout = AbortSignal.timeout(15_000)
  const response = await fetch(catalogURL, {
    credentials: 'omit',
    referrerPolicy: 'no-referrer',
    redirect: 'error',
    signal: signal ? AbortSignal.any([signal, timeout]) : timeout,
  })
  if (
    !response.ok ||
    !response.headers.get('content-type')?.startsWith('application/json') ||
    !response.body
  ) {
    throw new Error('Invalid models.dev catalog response')
  }
  const reader = response.body.getReader()
  const decoder = new TextDecoder()
  const chunks: string[] = []
  let size = 0
  try {
    for (;;) {
      const { value, done } = await reader.read()
      if (done) break
      size += value.byteLength
      if (size > catalogByteLimit) {
        await reader.cancel()
        throw new Error('Models.dev catalog exceeds its size limit')
      }
      chunks.push(decoder.decode(value, { stream: true }))
    }
    chunks.push(decoder.decode())
  } finally {
    reader.releaseLock()
  }
  return parse(chunks.join(''))
}

export type OfficialSourceQuote = {
  model: string
  source:
    | { kind: 'models_dev'; provider: string }
    | { kind: 'channel'; channel_id: number; upstream_model?: string }
  currency: 'USD'
  source_token_unit: 'per_million_tokens' | 'per_token'
  token_unit: 'per_million_tokens'
  rates: Record<string, string>
  context_tiers:
    | {
        above_input_tokens: number
        rates: Record<string, string>
      }[]
    | null
}

const tokenMeters = new Set([
  'input',
  'output',
  'cache_read',
  'cache_write',
  'reasoning',
  'input_audio',
  'output_audio',
])

function record(value: unknown): Record<string, unknown> | undefined {
  if (
    value === null ||
    typeof value !== 'object' ||
    Array.isArray(value) ||
    isLosslessNumber(value)
  ) {
    return
  }
  return value as Record<string, unknown>
}

function member(object: unknown, key: string) {
  const fields = record(object)
  return fields && Object.hasOwn(fields, key) ? fields[key] : undefined
}

// Convert scientific notation to plain decimal by moving the decimal point.
function rateDecimal(value: unknown): string {
  if (!isLosslessNumber(value) || value.value.length > 128) {
    throw new Error('invalid_source_price')
  }
  const parts = /^(\d+)(?:\.(\d+))?(?:[eE]([+-]?\d+))?$/.exec(value.value)
  if (!parts) throw new Error('invalid_source_price')
  const digits = parts[1] + (parts[2] ?? '')
  if (!/[1-9]/.test(digits)) return '0'
  const point = parts[1].length + Number(parts[3] ?? 0)
  if (point < -128 || point > 128) {
    throw new Error('unrepresentable_source_price')
  }
  const expanded =
    point <= 0
      ? `0.${'0'.repeat(-point)}${digits}`
      : point >= digits.length
        ? digits.padEnd(point, '0')
        : `${digits.slice(0, point)}.${digits.slice(point)}`
  const [whole, fraction = ''] = expanded.split('.')
  const tail = fraction.replace(/0+$/, '')
  return whole.replace(/^0+(?=\d)/, '') + (tail ? `.${tail}` : '')
}

function readRates(
  fields: Record<string, unknown>,
  metadata: string[]
): Record<string, string> {
  const rates: Record<string, string> = {}
  for (const [key, value] of Object.entries(fields)) {
    if (metadata.includes(key) || value === null) continue
    if (!tokenMeters.has(key)) throw new Error('unsupported_source_pricing')
    rates[key] = rateDecimal(value)
  }
  return rates
}

function readContext(
  fields: Record<string, unknown>
): OfficialSourceQuote['context_tiers'] {
  const levels = fields.tiers
  if (levels != null && !Array.isArray(levels)) {
    throw new Error('invalid_context_tiers')
  }
  if (!levels?.length && fields.context_over_200k != null) {
    // The compatibility field also appears for 272k and other thresholds.
    throw new Error('missing_context_threshold')
  }
  if (levels == null) return null
  if (levels.length > 3) throw new Error('unsupported_context_tier_count')
  const seen = new Set<number>()
  return levels
    .map((entry: unknown) => {
      const fields = record(entry)
      const tier = record(fields?.tier)
      if (
        !fields ||
        !tier ||
        tier.type !== 'context' ||
        Object.keys(tier).length !== 2
      ) {
        throw new Error('unsupported_context_condition')
      }
      if (!isLosslessNumber(tier.size) || !/^[1-9]\d*$/.test(tier.size.value)) {
        throw new Error('invalid_context_threshold')
      }
      const bound = Number(tier.size.value)
      if (!Number.isSafeInteger(bound) || seen.has(bound)) {
        throw new Error('invalid_context_threshold')
      }
      seen.add(bound)
      const rates = readRates(fields, ['tier'])
      if (!Object.keys(rates).length) throw new Error('invalid_context_tier')
      return { above_input_tokens: bound, rates }
    })
    .sort((a, b) => a.above_input_tokens - b.above_input_tokens)
}

/** Select exactly one provider and model; omitted meters have no source quote. */
export function officialModelsDevQuote(
  catalog: unknown,
  model: string,
  provider: string
): OfficialSourceQuote | null {
  const providerRow = member(catalog, provider)
  const price = member(member(member(providerRow, 'models'), model), 'cost')
  if (price == null) return null
  const fields = record(price)
  if (!fields) throw new Error('invalid_source_price')
  const rates = readRates(fields, ['tiers', 'context_over_200k'])
  const context = readContext(fields)
  if (!Object.keys(rates).length && !context?.length) return null
  return {
    model,
    source: { kind: 'models_dev', provider },
    currency: 'USD',
    source_token_unit: 'per_million_tokens',
    token_unit: 'per_million_tokens',
    rates,
    context_tiers: context,
  }
}

export async function fetchOfficialModelsDevQuote(
  model: string,
  provider: string,
  signal?: AbortSignal
): Promise<OfficialSourceQuote | null> {
  return officialModelsDevQuote(
    await fetchModelsDevCatalog(signal),
    model,
    provider
  )
}
