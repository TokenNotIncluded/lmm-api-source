/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import type { NavGroup, NavItem } from '@/components/layout/types'
import type { PluginInfo } from '@/features/plugins/api'

export const CAPABILITY_TTL_MS = 60_000
export const CAPABILITY_REFRESH_MS = 30_000

export type CapabilityEvidence<T> = {
  data: T | undefined
  dataUpdatedAt: number
  error?: unknown
  isError?: boolean
  isPending?: boolean
}

export type CapabilityPhase = 'pending' | 'fresh' | 'stale' | 'error'

/** Cached values remain readable, but only a recent successful read enables writes. */
export function capabilityPhase<T>(
  evidence: CapabilityEvidence<T>,
  now = Date.now()
): CapabilityPhase {
  if (evidence.isError || evidence.error) {
    return evidence.data === undefined ? 'error' : 'stale'
  }
  if (evidence.data === undefined || evidence.dataUpdatedAt <= 0) {
    return 'pending'
  }
  const age = now - evidence.dataUpdatedAt
  return age >= 0 && age < CAPABILITY_TTL_MS ? 'fresh' : 'stale'
}

export function moduleNavigationState(
  path: string,
  plugins: CapabilityEvidence<PluginInfo[]>,
  now = Date.now()
): 'unaffected' | 'unknown' | 'unavailable' | 'reported_enabled' {
  // Ownership comes from the host catalog, including unconfigured modules.
  // Until it has declared a path, there is no evidence to narrow that route.
  const owners = plugins.data?.filter((plugin) =>
    plugin.navigation_paths?.some(
      (prefix) => path === prefix || path.startsWith(`${prefix}/`)
    )
  )
  if (!owners?.length) return 'unaffected'
  if (capabilityPhase(plugins, now) !== 'fresh') return 'unknown'

  // A registry record is a declaration, never a permission or health grant.
  const registry = new Map(plugins.data?.map((plugin) => [plugin.name, plugin]))
  const checked = new Set<string>()
  const enabled = (name: string): boolean => {
    if (checked.has(name)) return true
    const plugin = registry.get(name)
    if (!plugin?.available || !plugin.installed || !plugin.enabled) return false
    checked.add(name)
    return (plugin.requires ?? []).every(enabled)
  }
  return owners.every((plugin) => enabled(plugin.name))
    ? 'reported_enabled'
    : 'unavailable'
}

/** This only narrows navigation that already passed role/configuration filtering. */
export function applyRuntimeNavigation(
  groups: NavGroup[],
  plugins: CapabilityEvidence<PluginInfo[]>,
  labels: { unknown: string; unavailable: string },
  now = Date.now()
): NavGroup[] {
  const narrow = <
    T extends { url?: unknown; disabled?: boolean; badge?: string },
  >(
    item: T
  ): T => {
    if (typeof item.url !== 'string') return item
    const state = moduleNavigationState(item.url, plugins, now)
    return state === 'unknown' || state === 'unavailable'
      ? { ...item, disabled: true, badge: labels[state] }
      : item
  }
  return groups.map((group) => ({
    ...group,
    items: group.items.map(
      (item): NavItem =>
        item.items ? { ...item, items: item.items.map(narrow) } : narrow(item)
    ),
  }))
}
