/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { isLosslessNumber, LosslessNumber } from 'lossless-json'
import { parse, stringify } from 'smol-toml'
import { z } from 'zod'

import { usdToMicros } from '@/features/channels/components/native-model-draft'
import {
  assertNativeIntegers,
  stringifyNativeDocument,
} from '@/features/channels/native-api'

import { decimalPriceFraction } from './native-price-rules'
import type { OfficialSourceQuote } from './official-models-dev-quote'

const rateUnit = 'micro-USD/1M tokens; request: micro-USD/request'
const tokenKeys = ['input', 'output', 'cache_read', 'cache_write'] as const
const tokenMeters = new Set<string>([
  ...tokenKeys,
  'cache_write_1h',
  'cache_write_12h',
])
const micros = z.number().nonnegative().int().safe()
const baseRates = z
  .object({
    input: micros,
    output: micros,
    cache_read: micros,
    cache_write: micros,
    request: micros,
  })
  .strict()
const exactInt = z.union([micros, z.instanceof(LosslessNumber)])
const fraction = z
  .object({ numerator: exactInt, denominator: exactInt })
  .strict()
const tier = z.object({
  up_to_input_tokens: micros.positive().nullish(),
  rates: z.record(z.string(), fraction),
})
const tariff = z
  .object({
    revision: micros,
    source_digest: z.string(),
    uses_cache_read: z.boolean(),
    uses_cache_write: z.boolean(),
    uses_cache_write_1h: z.boolean().optional(),
    uses_cache_write_12h: z.boolean().optional(),
    calendar: z
      .object({ match_tiers: z.array(tier) })
      .passthrough()
      .nullish(),
    tiers: z.array(tier),
  })
  .passthrough()
const price = z
  .object({
    source: z.string().min(1),
    source_unit: z.string().min(1),
    rates: baseRates,
    billing_mode: z.enum(['tokens', 'tiered_decimal']).optional(),
    tiered: tariff.optional(),
  })
  .strict()
type DraftPrice = Omit<z.infer<typeof price>, 'rates' | 'tiered'> & {
  rates: Partial<z.infer<typeof baseRates>>
  tiered?: z.infer<typeof tariff>
  unmapped_rates?: Record<string, string>
}
export type OfficialPrice = z.infer<typeof price>
export type OfficialCatalog = {
  models: Record<string, OfficialPrice>
  group_multipliers_ppm: Record<string, number>
  speed_multipliers_ppm: { fast: number; ultrafast: number }
}
const draft = price.extend({
  rates: baseRates.partial(),
  unmapped_rates: z.record(z.string(), z.string()).optional(),
})
const file = draft.extend({
  model: z.string(),
  currency: z.literal('USD'),
  rate_unit: z.literal(rateUnit),
})
function fromToml(value: unknown): unknown {
  if (typeof value === 'bigint') return new LosslessNumber(String(value))
  if (Array.isArray(value)) return value.map(fromToml)
  if (value && typeof value === 'object') {
    return Object.fromEntries(
      Object.entries(value).map(([k, v]) => [k, fromToml(v)])
    )
  }
  return value
}
// TOML's signed int64 range is smaller than Core's u64 fraction components.
// Quoted fraction integers outside the portable range round-trip exactly.
function forFile(value: unknown): unknown {
  if (isLosslessNumber(value)) return String(value)
  if (Array.isArray(value)) return value.map(forFile)
  if (value && typeof value === 'object') {
    return Object.fromEntries(
      Object.entries(value)
        .filter(([, v]) => v != null)
        .map(([k, v]) => [k, forFile(v)])
    )
  }
  return value
}
export function writeOfficialToml(model: string, value?: DraftPrice): string {
  const encoded = stringify(
    forFile({
      model,
      currency: 'USD',
      rate_unit: rateUnit,
      ...(value ?? {
        source: 'manual',
        source_unit: 'micro-USD/1M tokens',
        rates: {},
      }),
    })
  )
  return value
    ? encoded
    : `${
        encoded
      }# Enter integer micro-USD per million tokens for input, output, cache_read, cache_write.\n# request is micro-USD per request. Enter 0 only to confirm a free meter.\n# input = ...\n# output = ...\n# cache_read = ...\n# cache_write = ...\n# request = ...\n`
}
export function readOfficialToml(model: string, text: string): DraftPrice {
  if (new TextEncoder().encode(text).length > 256 * 1024) {
    throw new Error('Invalid model price file')
  }
  const raw = fromToml(
    parse(text, { integersAsBigInt: 'asNeeded', unsafeKeyBehaviour: 'throw' })
  ) as Record<string, unknown>
  // Only tariff fraction components can be decimal strings in a price file.
  const normalized = parseFractionStrings(raw)
  assertNativeIntegers(normalized)
  const { model: id, currency, rate_unit, ...value } = file.parse(normalized)
  if (id !== model || currency !== 'USD' || rate_unit !== rateUnit) {
    throw new Error('Model or rate units changed')
  }
  if (value.tiered) {
    for (const band of value.tiered.tiers) band.up_to_input_tokens ??= null
  }
  return value
}
function parseFractionStrings(value: unknown, path: string[] = []): unknown {
  if (
    typeof value === 'string' &&
    path.includes('tiered') &&
    path.at(-3) === 'rates' &&
    ['numerator', 'denominator'].includes(path.at(-1) ?? '') &&
    /^(0|[1-9]\d*)$/.test(value)
  ) {
    const n = BigInt(value)
    return n <= BigInt(Number.MAX_SAFE_INTEGER)
      ? Number(n)
      : new LosslessNumber(value)
  }
  if (isLosslessNumber(value)) return value
  if (Array.isArray(value)) {
    return value.map((v, i) => parseFractionStrings(v, [...path, String(i)]))
  }
  if (value && typeof value === 'object') {
    return Object.fromEntries(
      Object.entries(value).map(([k, v]) => [
        k,
        parseFractionStrings(v, [...path, k]),
      ])
    )
  }
  return value
}
export function completeOfficialPrice(value: DraftPrice): OfficialPrice {
  const p = price.parse(value)
  if (
    p.rates.request > 0 &&
    (p.tiered || tokenKeys.some((key) => p.rates[key] > 0))
  ) {
    throw new Error(
      'Choose one billing method. Set the request price to zero for token billing, or use a flat price with all token rates set to zero for per-request billing.'
    )
  }
  if ((p.billing_mode === 'tiered_decimal') !== Boolean(p.tiered)) {
    throw new Error('Incomplete context tariff')
  }
  if (
    p.tiered &&
    (Object.values(p.rates).some((rate) => rate !== 0) ||
      p.tiered.tiers.some((band) =>
        tokenKeys.some((key) => !Object.hasOwn(band.rates, key))
      ))
  ) {
    throw new Error('Incomplete context tariff')
  }
  return p
}
const fractionOf = (s: string) => {
  const rate = decimalPriceFraction(s)
  if (!rate) throw new Error('Invalid decimal price')
  return rate
}
function fromMicros(n: number) {
  return fractionOf(`${n}/1000000`)
}
function tierHash(value: unknown) {
  return crypto.subtle
    .digest(
      'SHA-256',
      new TextEncoder().encode(
        stringifyNativeDocument(value, undefined, ['tiered'])
      )
    )
    .then((bytes) =>
      Array.from(new Uint8Array(bytes), (v) =>
        v.toString(16).padStart(2, '0')
      ).join('')
    )
}
export async function mergeQuote(
  model: string,
  text: string,
  quote: OfficialSourceQuote
): Promise<string> {
  if (
    quote.model !== model ||
    quote.currency !== 'USD' ||
    quote.token_unit !== 'per_million_tokens'
  ) {
    throw new Error('Invalid model price source')
  }
  const previous = readOfficialToml(model, text)
  const source =
    quote.source.kind === 'channel'
      ? `channel/${quote.source.channel_id}`
      : ['models.dev', quote.source.provider, quote.source.source_model]
          .filter(Boolean)
          .join('/')
  const value: DraftPrice = {
    ...previous,
    rates: { ...previous.rates },
    source,
    source_unit:
      quote.source_token_unit === 'per_token' ? 'USD/token' : 'USD/1M tokens',
  }
  // Keep unrepresentable meters visible in the editable proposal. A reviewed
  // document containing these fields cannot be published as an executable price.
  const unmapped = { ...previous.unmapped_rates }
  const noteMeters = (rates: Record<string, string>, prefix = '') => {
    for (const [key, amount] of Object.entries(rates)) {
      if (!tokenMeters.has(key) && (key !== 'request' || prefix !== '')) {
        unmapped[prefix + key] = amount
      }
    }
  }
  noteMeters(quote.rates)
  quote.context_tiers?.forEach((t) =>
    noteMeters(t.rates, `${String(t.above_input_tokens)}/`)
  )
  if (Object.keys(unmapped).length) value.unmapped_rates = unmapped
  const requested = quote.rates.request
  if (requested !== undefined) {
    const rate = usdToMicros(requested)
    if (rate === null) throw new Error('Request price exceeds precision')
    value.rates.request = rate
  }
  const scalar =
    !previous.tiered &&
    !quote.context_tiers?.length &&
    Object.entries(quote.rates).every(
      ([k, v]) =>
        !tokenMeters.has(k) ||
        (tokenKeys.some((t) => t === k) && usdToMicros(v) !== null)
    )
  if (scalar) {
    for (const key of tokenKeys) {
      const v = quote.rates[key]
      if (v !== undefined) value.rates[key] = usdToMicros(v)!
    }
    return writeOfficialToml(model, value)
  }
  if (previous.tiered?.calendar) {
    throw new Error('Time-dependent prices require manual review')
  }
  const old = previous.tiered
  const oldTiers = old?.tiers ?? []
  const bounds =
    quote.context_tiers === null
      ? oldTiers.flatMap((t) =>
          t.up_to_input_tokens == null ? [] : [t.up_to_input_tokens]
        )
      : quote.context_tiers.map((t) => t.above_input_tokens)
  if (
    oldTiers.length > 1 &&
    quote.context_tiers !== null &&
    (oldTiers.length !== bounds.length + 1 ||
      bounds.some((bound, i) => oldTiers[i].up_to_input_tokens !== bound))
  ) {
    throw new Error('Changed context thresholds require manual review')
  }
  const flat = Object.fromEntries(
    tokenKeys.flatMap((key) =>
      previous.rates[key] === undefined
        ? []
        : [[key, fromMicros(previous.rates[key])]]
    )
  )
  const tiers = [...bounds, null].map((bound, i) => {
    const lower = i === 0 ? 0 : bounds[i - 1]
    const prior = oldTiers.find(
      (t) => t.up_to_input_tokens == null || t.up_to_input_tokens > lower
    )
    const rates = { ...(prior?.rates ?? flat) }
    // Without a source schedule apply a base refresh to the base band alone.
    // All existing higher-context bands keep their independent prices.
    const incoming =
      i === 0 || quote.context_tiers !== null
        ? { ...quote.rates, ...(i ? quote.context_tiers?.[i - 1]?.rates : {}) }
        : {}
    for (const [key, v] of Object.entries(incoming)) {
      if (tokenMeters.has(key)) rates[key] = fractionOf(v)
    }
    return { up_to_input_tokens: bound, rates }
  })
  const sourceWrites = (k: string) =>
    Object.hasOwn(quote.rates, k) ||
    quote.context_tiers?.some((t) => Object.hasOwn(t.rates, k)) === true
  const write1h =
    old?.uses_cache_write_1h === true || sourceWrites('cache_write_1h')
  const write12h =
    old?.uses_cache_write_12h === true || sourceWrites('cache_write_12h')
  // Flat drafts distinguish a confirmed free meter from a missing price.
  // Tiered drafts store zeros in their scalar fields for wire compatibility.
  const cacheMeter = (key: 'cache_read' | 'cache_write', enabled: boolean) =>
    enabled || sourceWrites(key) || (!old && previous.rates[key] !== undefined)
  const read = cacheMeter('cache_read', old?.uses_cache_read === true)
  const write = cacheMeter('cache_write', old?.uses_cache_write === true)
  const revision = write12h
    ? 4
    : old?.revision === 3
      ? 3
      : write || write1h || old?.revision === 2
        ? 2
        : 1
  value.billing_mode = 'tiered_decimal'
  const tariff = {
    ...old,
    revision,
    uses_cache_read: read,
    uses_cache_write: write,
    uses_cache_write_1h: write1h,
    uses_cache_write_12h: write12h,
    tiers,
  }
  value.tiered = {
    ...tariff,
    source_digest: await tierHash({
      source,
      revision,
      uses_cache_read: read,
      uses_cache_write: write,
      uses_cache_write_1h: write1h,
      uses_cache_write_12h: write12h,
      tiers,
    }),
  }
  // Scalar fields are the native tiered-mode sentinel. Only the operator
  // or an actual per-request source may confirm an unknown request price.
  value.rates = {
    input: 0,
    output: 0,
    cache_read: 0,
    cache_write: 0,
    ...(value.rates.request !== undefined
      ? { request: value.rates.request }
      : {}),
  }
  return writeOfficialToml(model, value)
}
