/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { LosslessNumber } from 'lossless-json'
import { z } from 'zod'

import { microsToUsd } from '@/features/channels/components/native-model-draft'
import {
  parseNativeDocument,
  stringifyNativeDocument,
  type NativePrices,
} from '@/features/channels/native-api'

export const nativePriceRuleSchema = z
  .object({
    id: z.number().int().positive().safe(),
    model: z.string().trim().min(1).max(200),
    group: z.string().trim().min(1),
    speed: z.enum(['standard', 'fast', 'ultrafast']),
    billing_mode: z
      .enum(['tokens', 'upstream_cost', 'tiered_decimal'])
      .optional(),
    rates: z.object({
      input: z.number().int().nonnegative().safe(),
      output: z.number().int().nonnegative().safe(),
      cache_read: z.number().int().nonnegative().safe(),
      cache_write: z.number().int().nonnegative().safe(),
      request: z.number().int().nonnegative().safe(),
    }),
    group_multiplier_ppm: z.number().int().min(0).max(1_000_000_000),
    speed_multiplier_ppm: z.number().int().min(1).max(1_000_000_000),
  })
  .passthrough()

export type NativePriceRule = z.infer<typeof nativePriceRuleSchema>
export const priceRateKeys = [
  'input',
  'output',
  'cache_read',
  'cache_write',
  'request',
] as const
export type PriceRateKey = (typeof priceRateKeys)[number]

export function readPriceRules(prices: NativePrices): NativePriceRule[] {
  const rules = z.array(nativePriceRuleSchema).parse(prices.rules)
  const ids = new Set<number>()
  const selectors = new Set<string>()
  for (const rule of rules) {
    const selector = JSON.stringify([rule.model, rule.group, rule.speed])
    if (ids.has(rule.id) || selectors.has(selector)) {
      throw new Error(
        'Each price rule needs a unique ID and model, group and speed combination.'
      )
    }
    ids.add(rule.id)
    selectors.add(selector)
  }
  return rules
}

export function readPriceDocument(
  document: string,
  version: number
): NativePrices {
  const value = parseNativeDocument(document)
  const parsed = z
    .object({
      version: z.number().int().positive().safe().optional(),
      rules: z.array(z.record(z.string(), z.unknown())),
    })
    .strict()
    .parse(value)
  const prices = { version, rules: parsed.rules }
  readPriceRules(prices)
  return prices
}

export function nextPriceRuleId(prices: NativePrices): number {
  const id =
    prices.rules.reduce(
      (highest, rule) => Math.max(highest, Number(rule.id)),
      0
    ) + 1
  if (!Number.isSafeInteger(id)) {
    throw new Error('No safe price rule ID is available.')
  }
  return id
}

/** Integer rates stay exact; a zero placeholder never represents a tiered price. */
export function priceRateText(
  rule: NativePriceRule,
  key: PriceRateKey
): string | null {
  if (rule.billing_mode === 'upstream_cost') return null
  if (rule.billing_mode !== 'tiered_decimal') {
    return microsToUsd(rule.rates[key])
  }
  if (key === 'request') return null
  const tariff = rule.tiered as
    | { tiers?: Array<{ rates?: Record<string, unknown> }> }
    | undefined
  const values =
    tariff?.tiers?.map((tier) => fractionText(tier.rates?.[key])) ?? []
  if (!values.length || values.some((value) => value === null)) return null
  return [...new Set(values)].join(' / ')
}

export function fractionText(value: unknown): string | null {
  if (!value || typeof value !== 'object') return null
  const fraction = value as { numerator?: unknown; denominator?: unknown }
  const numerator = String(fraction.numerator)
  const denominator = String(fraction.denominator)
  if (!/^\d+$/.test(numerator) || !/^[1-9]\d*$/.test(denominator)) return null
  const n = BigInt(numerator)
  const d = BigInt(denominator)
  let remainder = n % d
  let decimals = ''
  while (remainder !== 0n && decimals.length < 18) {
    remainder *= 10n
    decimals += String(remainder / d)
    remainder %= d
  }
  if (remainder !== 0n) return `${numerator}/${denominator}`
  return `${n / d}${decimals ? `.${decimals}` : ''}`
}

/** Decimal tariff inputs use exact fractions, including values outside JS's safe integer range. */
export function decimalPriceFraction(value: string): {
  numerator: number | LosslessNumber
  denominator: number | LosslessNumber
} | null {
  if (!/^\d+(?:\.\d{1,18})?$/.test(value) && !/^\d+\/[1-9]\d*$/.test(value)) {
    return null
  }
  const [whole, fraction = ''] = value.split('.')
  let numerator = value.includes('/')
    ? BigInt(value.split('/')[0])
    : BigInt(whole + fraction)
  let denominator = value.includes('/')
    ? BigInt(value.split('/')[1])
    : 10n ** BigInt(fraction.length)
  let a = numerator
  let b = denominator
  while (b) [a, b] = [b, a % b]
  numerator /= a
  denominator /= a
  if (
    numerator > 18_446_744_073_709_551_615n ||
    denominator > 1_000_000_000_000_000_000n
  ) {
    return null
  }
  const exact = (integer: bigint) =>
    integer <= BigInt(Number.MAX_SAFE_INTEGER)
      ? Number(integer)
      : new LosslessNumber(String(integer))
  return { numerator: exact(numerator), denominator: exact(denominator) }
}

export function priceChanges(before: NativePrices, after: NativePrices) {
  const previous = new Map(before.rules.map((rule) => [rule.id, rule]))
  const next = new Map(after.rules.map((rule) => [rule.id, rule]))
  let added = 0
  let changed = 0
  for (const [id, rule] of next) {
    if (!previous.has(id)) added++
    else if (!samePriceRule(previous.get(id), rule)) changed++
  }
  return {
    added,
    changed,
    removed: [...previous.keys()].filter((id) => !next.has(id)).length,
  }
}

export function samePriceRule(before: unknown, after: unknown): boolean {
  function ordered(value: unknown): unknown {
    if (Array.isArray(value)) return value.map(ordered)
    if (
      value !== null &&
      typeof value === 'object' &&
      !(value instanceof LosslessNumber)
    ) {
      return Object.fromEntries(
        Object.entries(value)
          .sort(([left], [right]) => left.localeCompare(right))
          .map(([key, item]) => [key, ordered(item)])
      )
    }
    return value
  }
  return (
    stringifyNativeDocument(ordered(before ?? {})) ===
    stringifyNativeDocument(ordered(after ?? {}))
  )
}
