/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import type { NativeCacheWrite12hTier } from '../catalog-contract'

/** Up to 18 fractional digits fit the public price display. */
export function exactRateDecimal(
  numerator: bigint,
  denominator: bigint
): string | null {
  let remainder = numerator % denominator
  let decimals = ''
  for (let index = 0; remainder !== 0n && index < 18; index++) {
    remainder *= 10n
    decimals += (remainder / denominator).toString()
    remainder %= denominator
  }
  return remainder === 0n
    ? `${numerator / denominator}${decimals ? `.${decimals}` : ''}`
    : null
}

/** Display the exact rate; never round a small positive price to zero. */
export function formatCacheWriteRate(
  rate: NativeCacheWrite12hTier['input']
): string {
  const exact = exactRateDecimal(
    BigInt(rate.numerator),
    BigInt(rate.denominator)
  )
  return `${exact ?? `${rate.numerator}/${rate.denominator}`} USD`
}
