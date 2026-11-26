/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import type { NativeConfiguration } from '@/features/channels/native-api'

import type { NativePriceRule } from './native-price-rules'
import { ROUTER_MODELS } from './router-price-draft'

export type PriceSelector = Pick<NativePriceRule, 'model' | 'group' | 'speed'>

export function priceSelectorKey(selector: PriceSelector): string {
  return JSON.stringify([selector.model, selector.group, selector.speed])
}

/**
 * Standard pricing needs an enabled route and target. Premium speed flags
 * express configuration intent; Core does not yet prove premium settlement.
 */
export function configuredPriceSpeeds(
  routing: NativeConfiguration['routing'],
  model: string,
  group: string
): PriceSelector['speed'][] {
  if (!routing.groups.some((item) => item.name === group && item.enabled))
    return []
  const enabledUpstreams = new Set(
    routing.upstreams.filter((item) => item.enabled).map((item) => item.id)
  )
  const configured: PriceSelector['speed'][] = []
  for (const route of routing.routes) {
    if (!route.enabled || route.model !== model || route.group !== group)
      continue
    for (const speed of route.speeds) {
      if (speed !== 'standard') continue
      if (configured.includes(speed)) continue
      if (
        route.targets.some(
          (target) =>
            target.enabled &&
            enabledUpstreams.has(target.upstream_id) &&
            target.speeds.includes(speed)
        )
      )
        configured.push(speed)
    }
  }
  return configured
}

// Core's current budget contract permits verified loopback providers only.
function isLoopbackEndpoint(endpoint: string): boolean {
  try {
    const host = new URL(endpoint).hostname
    return host === '[::1]' || /^127(?:\.\d{1,3}){3}$/.test(host)
  } catch {
    return false
  }
}

export function configuredRouterSelectors(
  routing: NativeConfiguration['routing']
): PriceSelector[] {
  const upstreams = new Map(
    routing.upstreams
      .filter((upstream) => upstream.enabled)
      .map((upstream) => [upstream.id, upstream])
  )
  const groups = new Set(
    routing.groups.filter((item) => item.enabled).map((item) => item.name)
  )
  return routing.routes.flatMap((route) => {
    if (
      !route.enabled ||
      !groups.has(route.group) ||
      !route.speeds.includes('standard') ||
      !ROUTER_MODELS.some((model) => model === route.model)
    )
      return []
    const targets = route.targets.filter(
      (target) =>
        target.enabled &&
        target.speeds.includes('standard') &&
        upstreams.has(target.upstream_id)
    )
    if (
      !targets.length ||
      !targets.every((target) =>
        isLoopbackEndpoint(upstreams.get(target.upstream_id)?.endpoint ?? '')
      )
    )
      return []
    return [{ model: route.model, group: route.group, speed: 'standard' }]
  })
}

export const toolProtocols = {
  'deepseek-search': 'deepseek_search',
  'deepseek-fetch': 'deepseek_fetch',
} as const

export function isToolFeeRule(rule: NativePriceRule): boolean {
  return (
    (!rule.billing_mode || rule.billing_mode === 'tokens') &&
    rule.tiered === undefined &&
    rule.rates.input === 0 &&
    rule.rates.output === 0 &&
    rule.rates.cache_read === 0 &&
    rule.rates.cache_write === 0
  )
}

export function routingPriceCoverage(
  routing: NativeConfiguration['routing'],
  rules: NativePriceRule[]
) {
  const bySelector = new Map(
    rules.map((rule) => [priceSelectorKey(rule), rule])
  )
  const upstreams = new Map(
    routing.upstreams
      .filter((item) => item.enabled)
      .map((item) => [item.id, item.protocol])
  )
  const missing: PriceSelector[] = []
  const tools: Array<{ selector: PriceSelector; rule?: NativePriceRule }> = []
  const seen = new Set<string>()
  const enabledGroups = new Set(
    routing.groups.filter((item) => item.enabled).map((item) => item.name)
  )
  for (const route of routing.routes) {
    if (!route.enabled || !enabledGroups.has(route.group)) continue
    // These are the two per-successful-request protocols supported by Core.
    const toolProtocol = Object.hasOwn(toolProtocols, route.model)
      ? toolProtocols[route.model as keyof typeof toolProtocols]
      : undefined
    for (const speed of route.speeds) {
      if (speed !== 'standard') {
        continue
      }
      const targets = route.targets.filter(
        (target) =>
          target.enabled &&
          upstreams.has(target.upstream_id) &&
          target.speeds.includes(speed)
      )
      if (!targets.length) continue
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
        targets.some(
          (target) => upstreams.get(target.upstream_id) === toolProtocol
        )
      ) {
        tools.push({
          selector,
          rule: rule && isToolFeeRule(rule) ? rule : undefined,
        })
      }
    }
  }
  return { missing, tools }
}
