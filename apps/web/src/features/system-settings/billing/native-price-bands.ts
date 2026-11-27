/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import {
  hasDecimalPriceRates,
  priceRatePair,
} from '@/features/channels/lib/native-price-display'

import { type NativePriceRule } from './native-price-rules'

const meters = [
  'input',
  'output',
  'cache_read',
  'cache_write',
  'cache_write_1h',
  'cache_write_12h',
  'request',
] as const
type Meter = (typeof meters)[number]
export type BandPrice = {
  meter: Meter
  state: 'rate' | 'missing' | 'unused' | 'target'
  base: string | null
  final: string | null
}
export type PriceBand = {
  branch: 'standard' | 'calendar'
  lower: string | null
  upper: string | null
  prices: BandPrice[]
}

function record(value: unknown): Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {}
}

export function publishedPriceBands(rule: NativePriceRule) {
  const tariff = record(rule.tiered)
  const calendar = record(tariff.calendar)
  const bands: PriceBand[] = []
  const decimal = hasDecimalPriceRates(rule.billing_mode)
  function add(rows: unknown, branch: PriceBand['branch']) {
    let lower: string | null = null
    if (!Array.isArray(rows)) return
    for (const value of rows) {
      const band = record(value)
      const rates = record(band.rates)
      const upper =
        band.up_to_input_tokens == null ? null : String(band.up_to_input_tokens)
      const prices = meters.map((meter): BandPrice => {
        const lifetime =
          meter === 'cache_write_1h' || meter === 'cache_write_12h'
        const cache = meter.startsWith('cache_')
        const targetBound = rule.billing_mode === 'official_flat' && lifetime
        const disabled = decimal
          ? meter === 'request' ||
            (cache &&
              (tariff['uses_' + meter] === false ||
                (lifetime && tariff['uses_' + meter] === undefined)))
          : lifetime
        const pair =
          disabled || targetBound
            ? null
            : priceRatePair(
                rates[meter],
                decimal,
                rule.group_multiplier_ppm,
                rule.speed_multiplier_ppm
              )
        return {
          meter,
          state: targetBound
            ? 'target'
            : disabled
              ? 'unused'
              : pair
                ? 'rate'
                : 'missing',
          base: pair?.base ?? null,
          final: pair?.final ?? null,
        }
      })
      bands.push({ branch, lower, upper, prices })
      lower = upper
    }
  }
  if (rule.billing_mode !== 'upstream_cost') {
    if (decimal) {
      add(tariff.tiers, 'standard')
      add(calendar.match_tiers, 'calendar')
    } else {
      add([{ rates: rule.rates }], 'standard')
    }
  }
  return {
    bands,
    hasCalendar: tariff.calendar != null,
    calendarExpression:
      typeof calendar.source_expression === 'string'
        ? calendar.source_expression
        : null,
    digest:
      typeof tariff.source_digest === 'string' ? tariff.source_digest : null,
    revision: tariff.revision,
  }
}
