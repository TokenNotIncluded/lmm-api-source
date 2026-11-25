/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import type { NativeConfiguration } from '@/features/channels/native-api'

import type { NativePriceRule } from './native-price-rules'

export type PriceSelector = Pick<NativePriceRule, 'model' | 'group' | 'speed'>

export function priceSelectorKey(selector: PriceSelector): string {
  return JSON.stringify([selector.model, selector.group, selector.speed])
}

export function routingPriceCoverage(
  routing: NativeConfiguration['routing'],
  rules: NativePriceRule[]
) {
  const bySelector = new Map(
    rules.map((rule) => [priceSelectorKey(rule), rule])
  )
  const upstreams = new Map(
    routing.upstreams.map((item) => [item.id, item.protocol])
  )
  const missing: PriceSelector[] = []
  const tools: Array<{ selector: PriceSelector; rule?: NativePriceRule }> = []
  const seen = new Set<string>()
  for (const route of routing.routes) {
    // These are the two per-successful-request protocols supported by Core.
    const toolProtocol =
      route.model === 'deepseek-search'
        ? 'deepseek_search'
        : route.model === 'deepseek-fetch'
          ? 'deepseek_fetch'
          : undefined
    for (const speed of route.speeds) {
      if (speed !== 'standard' && speed !== 'fast' && speed !== 'ultrafast') {
        continue
      }
      const selector = {
        model: route.model,
        group: route.group,
        speed,
      } satisfies PriceSelector
      const key = priceSelectorKey(selector)
      if (seen.has(key)) continue
      seen.add(key)
      const rule = bySelector.get(key)
      if (!rule) missing.push(selector)
      if (
        toolProtocol &&
        route.targets.some(
          (target) =>
            upstreams.get(target.upstream_id) === toolProtocol &&
            target.speeds.includes(speed)
        )
      ) {
        tools.push({ selector, rule })
      }
    }
  }
  return { missing, tools }
}
