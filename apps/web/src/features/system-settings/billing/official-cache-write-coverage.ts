/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import type { NativeConfiguration } from '@/features/channels/native-api'

import type { OfficialPrice } from './official-price-toml'

type Destination = { protocol: string; payload?: unknown }

export function officialPriceDestinations(
  routing: NativeConfiguration['routing'],
  model: string,
  pendingChannel?: number,
  sourceChannel?: number
): Destination[] {
  const channels = new Map(
    routing.upstreams.map((channel) => [channel.id, channel])
  )
  const destinations: Destination[] = []
  let hasPendingTarget = false
  for (const route of routing.routes) {
    if (route.model !== model) continue
    for (const target of route.targets) {
      const channel = channels.get(target.upstream_id)
      if (
        !channel ||
        !target.speeds.some((speed) => route.speeds.includes(speed))
      ) {
        continue
      }
      destinations.push({ protocol: channel.protocol, payload: target.payload })
      if (channel.id === pendingChannel) hasPendingTarget = true
    }
  }
  const hint =
    pendingChannel ?? (destinations.length === 0 ? sourceChannel : undefined)
  const channel = hint === undefined ? undefined : channels.get(hint)
  if (channel && !hasPendingTarget) {
    destinations.push({ protocol: channel.protocol })
  }
  return destinations
}

export function cacheWriteTariffTarget(
  price: Pick<OfficialPrice, 'tiered'> | undefined,
  destinations: Destination[]
) {
  const tariff = price?.tiered
  if (!tariff || (tariff.revision === 1 && !tariff.uses_cache_write)) return
  const protocols = new Set(
    destinations.map(({ protocol, payload }) =>
      protocol === 'openai_chat' &&
      payload === 'legacy_openrouter_chat_usage_v1'
        ? 'legacy_openrouter_chat_usage_v1'
        : protocol
    )
  )
  const openai =
    protocols.has('openai_chat') || protocols.has('openai_responses')
  const legacy = protocols.has('legacy_openrouter_chat_usage_v1')
  const anthropic = protocols.has('anthropic_messages')
  if (!openai && !legacy && !anthropic) return
  if ((openai && (legacy || anthropic)) || protocols.has('gemini')) {
    return 'mixed'
  }
  if (openai) return 'openai'
  return legacy ? 'legacy' : 'anthropic'
}

export function cacheWriteCoverageError(
  price: Pick<OfficialPrice, 'tiered'> | undefined,
  destinations: Destination[]
): string | undefined {
  const tariff = price?.tiered
  const target = cacheWriteTariffTarget(price, destinations)
  if (!tariff || !target) return
  if (target === 'mixed') {
    return 'The configured protocols cannot share this tiered cache-write tariff. Review the model’s route targets before publishing. Keep the draft until each target can use the same confirmed rates.'
  }
  const bands = [...tariff.tiers, ...(tariff.calendar?.match_tiers ?? [])]
  const hasRate = (name: string) =>
    bands.every((band) => Object.hasOwn(band.rates, name))
  if (
    target === 'openai' &&
    (tariff.revision !== 4 ||
      !tariff.uses_cache_read ||
      !tariff.uses_cache_write ||
      tariff.uses_cache_write_12h !== true ||
      tariff.uses_cache_write_1h === true ||
      !hasRate('cache_write_12h'))
  ) {
    return 'OpenAI uses cache_write for 30-minute writes. Set tiered.revision to 4, enable uses_cache_read, uses_cache_write and uses_cache_write_12h, and disable uses_cache_write_1h. Enter rates.cache_write_12h in every tier, including calendar.match_tiers. Enter { numerator = 0, denominator = 1 } only for a confirmed free 12-hour write. Keep unknown prices in the draft.'
  }
  if (
    target === 'legacy' &&
    (tariff.revision !== 3 ||
      !tariff.uses_cache_write ||
      tariff.uses_cache_write_1h !== true ||
      !hasRate('cache_write_1h'))
  ) {
    return 'Legacy OpenRouter Chat requires revision 3 with one unbounded tier, uses_cache_write_1h enabled, and an explicit rates.cache_write_1h. Enter { numerator = 0, denominator = 1 } only for a confirmed free 1-hour write. Keep unknown prices in the draft.'
  }
  if (
    target === 'anthropic' &&
    ((tariff.revision !== 2 && tariff.revision !== 3) ||
      !tariff.uses_cache_write ||
      tariff.uses_cache_write_1h !== true ||
      !hasRate('cache_write_1h'))
  ) {
    return 'Anthropic uses cache_write for 5-minute writes. For a context tariff, use revision 2, enable uses_cache_write and uses_cache_write_1h, and enter rates.cache_write_1h in every tier, including calendar.match_tiers. Enter { numerator = 0, denominator = 1 } only for a confirmed free 1-hour write. Keep unknown prices in the draft.'
  }
}
