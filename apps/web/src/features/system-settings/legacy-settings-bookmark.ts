/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { redirect } from '@tanstack/react-router'

import { legacySettingsDestination } from './legacy-settings-destination'

export function legacyBookmarkSection(
  search: unknown,
  fallback: string
): string {
  if (
    typeof search === 'object' &&
    search !== null &&
    'section' in search &&
    typeof search.section === 'string'
  ) {
    return search.section
  }
  return fallback
}

export function resolveLegacySettingsBookmark<T extends string>(
  category: string,
  section: string,
  meta: (id: T) => { titleKey: string }
) {
  const alias = category === 'operations' && section === 'monitoring'
  const destination = alias
    ? legacySettingsDestination('models', 'routing-reliability')
    : legacySettingsDestination(category, section)
  if (destination?.action === 'redirect') {
    throw redirect({ href: destination.href, replace: true })
  }
  return {
    category,
    section,
    // Only a registered section can call its registry's title lookup.
    titleKey: destination ? meta(section as T).titleKey : 'Page not found',
    destination: destination ?? null,
  }
}

export type LegacySettingsBookmark = ReturnType<
  typeof resolveLegacySettingsBookmark
>
