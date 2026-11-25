/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.
*/
import type { TFunction } from 'i18next'

import { getSystemSettingsNavGroups } from '@/components/layout/config/system-settings.config'
import type { NavGroup } from '@/components/layout/types'
import { ROLE } from '@/lib/roles'

/**
 * One jumpable destination inside `/system-settings/*`.
 *
 * `group` is the admin section shown in the sidebar ("Billing & Payment") and
 * `section` is the page inside it ("Quota Settings"). Both are localized
 * strings, so the index has to be rebuilt when the interface language changes.
 */
export type SettingsSearchEntry = {
  group: string
  section: string
  url: string
  /** Extra search terms, lowercased. */
  keywords: string[]
}

export type SettingsSearchableGroup = SettingsSearchEntry & {
  /** Entries of the same admin section, for grouped rendering. */
  entries: SettingsSearchEntry[]
}

function normalize(value: string) {
  return value.trim().toLowerCase()
}

/** Flatten the localized settings navigation into a searchable index. */
export function buildSettingsSearchIndex(
  t: TFunction,
  groups: NavGroup[] | undefined = undefined,
  role: number = ROLE.SUPER_ADMIN
): SettingsSearchableGroup[] {
  return (groups ?? getSystemSettingsNavGroups(t, role)).map((group) => {
    const entries = group.items.flatMap((item) =>
      item.url
        ? [
            {
              group: group.title,
              section: item.title,
              url: item.url,
              keywords: [group.title, item.title].map(normalize),
            },
          ]
        : []
    )
    return {
      group: group.title,
      section: group.title,
      url: entries[0]?.url ?? '',
      keywords: [normalize(group.title)],
      entries,
    }
  })
}

function scoreEntry(entry: SettingsSearchEntry, query: string): number {
  const needle = normalize(query)
  if (!needle) return 1
  const section = normalize(entry.section)
  const group = normalize(entry.group)
  if (section.startsWith(needle)) return 100
  if (section.includes(needle)) return 70
  if (group.includes(needle)) return 50
  if (entry.keywords.some((keyword) => keyword.includes(needle))) return 30
  if (entry.url.toLowerCase().includes(needle)) return 20
  return 0
}

/**
 * Rank the index for a free-text query, best match first.
 *
 * An empty query returns the whole index so the palette doubles as a browsable
 * table of contents.
 */
export function searchSettings(
  index: SettingsSearchableGroup[],
  query: string
): SettingsSearchableGroup[] {
  if (!query.trim()) return index
  return index
    .map((group) => {
      const scored = group.entries
        .map((entry) => ({ entry, score: scoreEntry(entry, query) }))
        .filter((item) => item.score > 0)
        .sort((a, b) => b.score - a.score)
      return { group, scored }
    })
    .filter((item) => item.scored.length > 0)
    .sort(
      (a, b) =>
        (b.scored[0]?.score ?? 0) - (a.scored[0]?.score ?? 0) ||
        a.group.section.localeCompare(b.group.section)
    )
    .map((item) => ({
      ...item.group,
      entries: item.scored.map((entry) => entry.entry),
    }))
}

/** Total number of jumpable settings destinations. */
export function countSettingsEntries(index: SettingsSearchableGroup[]) {
  return index.reduce((total, group) => total + group.entries.length, 0)
}
