/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import type { NativeCacheWrite12hTier } from '../catalog-contract'

/** Display the exact rate; never round a small positive price to zero. */
export function formatCacheWriteRate(
  rate: NativeCacheWrite12hTier['input']
): string {
  const numerator = BigInt(rate.numerator)
  const denominator = BigInt(rate.denominator)
  let remainder = numerator % denominator
  let decimals = ''
  for (let index = 0; remainder !== 0n && index < 18; index++) {
    remainder *= 10n
    decimals += (remainder / denominator).toString()
    remainder %= denominator
  }
  const value =
    remainder === 0n
      ? `${numerator / denominator}${decimals ? `.${decimals}` : ''}`
      : `${numerator}/${denominator}`
  return `${value} USD`
}
