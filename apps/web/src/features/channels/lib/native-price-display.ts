/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { LosslessNumber } from 'lossless-json'

function record(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null
}
function integer(value: unknown): bigint | null {
  if (typeof value === 'number' && Number.isSafeInteger(value) && value >= 0) {
    return BigInt(value)
  }
  if (value instanceof LosslessNumber && /^(0|[1-9]\d*)$/.test(value.value)) {
    return BigInt(value.value)
  }
  return null
}

/** Exact decimal where finite; an exact reduced fraction otherwise. */
function fraction(numerator: bigint, denominator: bigint): string {
  let a = numerator
  let b = denominator
  while (b !== 0n) [a, b] = [b, a % b]
  numerator /= a
  denominator /= a
  const whole = numerator / denominator
  let remainder = numerator % denominator
  let decimals = ''
  for (let i = 0; remainder !== 0n && i < 36; i++) {
    remainder *= 10n
    decimals += (remainder / denominator).toString()
    remainder %= denominator
  }
  return remainder === 0n
    ? `${whole}${decimals ? `.${decimals}` : ''}`
    : `${numerator}/${denominator}`
}

export function hasDecimalPriceRates(mode: unknown) {
  return mode === 'tiered_decimal' || mode === 'official_flat'
}

/** Configured USD / 1M token rates, or USD / request, including both ppm multipliers.
 * This is a catalog display, not a settlement calculation or a user quote.
 */
export function nativeConfiguredPrice(
  rule: Record<string, unknown>,
  dimension: 'input' | 'output' | 'cache_read' | 'cache_write' | 'request',
  tierIndex = 0
): string | null {
  const decimal = hasDecimalPriceRates(rule.billing_mode)
  if (
    rule.billing_mode !== undefined &&
    rule.billing_mode !== 'tokens' &&
    !decimal
  ) {
    return null
  }
  if (!decimal && rule.tiered != null) return null
  const group = integer(rule.group_multiplier_ppm)
  const speed = integer(rule.speed_multiplier_ppm)
  if (group === null || speed === null) return null
  let numerator: bigint | null
  let denominator: bigint | null
  if (decimal) {
    const tariff = record(rule.tiered)
    // A time-dependent tariff cannot be presented as one unconditional price.
    if (!tariff || tariff.calendar || !Array.isArray(tariff.tiers)) return null
    const tier = record(tariff.tiers[tierIndex])
    const rate = record(record(tier?.rates)?.[dimension])
    numerator = integer(rate?.numerator)
    denominator = integer(rate?.denominator)
  } else {
    numerator = integer(record(rule.rates)?.[dimension])
    denominator = 1_000_000n
  }
  if (numerator === null || denominator === null || denominator === 0n) {
    return null
  }
  return fraction(numerator * group * speed, denominator * 1_000_000_000_000n)
}
