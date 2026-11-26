/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.

*/
import type { NavGroup, NavItem, NavLink, TopNavLink } from '../types'

// Promote only links that have already passed role and sidebar-module filters.
// Do not reconstruct entries here: badges, aliases and panel interactions matter.
const PRIMARY_URLS = [
  '/dashboard/overview',
  '/pricing',
  '/keys',
  '/usage-logs/common',
  '/wallet',
] as const

const MANAGEMENT_URLS = [
  '/channels',
  '/users',
  '/system-settings',
  '/system-settings/site',
] as const

export function organizeConsoleNavigation(
  groups: NavGroup[],
  title: string,
  moreAdminTitle?: string
) {
  // First-run navigation is intentionally separate from the activated console.
  if (groups.some((group) => group.id === 'onboarding')) return groups

  const primary: NavItem[] = []
  const promoted = new Set<NavItem>()
  for (const url of PRIMARY_URLS) {
    const item = groups
      .flatMap((group) => group.items)
      .find((entry) => entry.url === url)
    if (item) {
      primary.push(item)
      promoted.add(item)
    }
  }

  const admin = groups.find((group) => group.id === 'admin')
  const management: NavItem[] = []
  for (const url of MANAGEMENT_URLS) {
    const item = admin?.items.find((entry) => entry.url === url)
    if (item) {
      management.push(item)
      promoted.add(item)
    }
  }

  const remaining = groups
    .map((group) => ({
      ...group,
      title:
        group.id === 'admin' && management.length && moreAdminTitle
          ? moreAdminTitle
          : group.title,
      items: group.items.filter((item) => !promoted.has(item)),
    }))
    .filter((group) => group.items.length > 0)

  return [
    ...(primary.length
      ? [{ id: 'console-primary', title, items: primary }]
      : []),
    ...(management.length && admin
      ? [{ id: 'console-management', title: admin.title, items: management }]
      : []),
    ...remaining,
  ]
}

function leafItems(groups: NavGroup[]) {
  return groups.flatMap((group) =>
    group.items.flatMap((item) => (item.items ? item.items : [item]))
  )
}

export function findConsoleNavItem(groups: NavGroup[], href: string) {
  const pathname = href.split(/[?#]/)[0].replace(/\/+$/, '') || '/'
  let match: NavLink | undefined
  let matchedLength = -1

  for (const group of groups) {
    for (const parent of group.items) {
      for (const item of parent.items ?? [parent]) {
        if (!item.url) continue
        for (const url of [item.url, ...(item.activeUrls ?? [])]) {
          if (typeof url !== 'string') continue
          const base = url.split(/[?#]/)[0].replace(/\/+$/, '') || '/'
          if (
            base.length > matchedLength &&
            (pathname === base ||
              (base !== '/' && pathname.startsWith(`${base}/`)))
          ) {
            match = parent.disabled ? { ...item, disabled: true } : item
            matchedLength = base.length
          }
        }
      }
    }
  }

  return match
}

export function getConsolePageTitle(groups: NavGroup[], href: string) {
  return findConsoleNavItem(groups, href)?.title
}

export function getConsoleSiteLinks(links: TopNavLink[], groups: NavGroup[]) {
  const destinations = new Set(leafItems(groups).map((item) => item.url))
  const seen = new Set<string>()
  return links.filter((link) => {
    // Keep explicitly external and disabled custom links with their semantics.
    if (!link.external && !link.disabled && destinations.has(link.href)) {
      return false
    }
    const key = `${Boolean(link.external)}:${link.href}`
    if (seen.has(key)) return false
    seen.add(key)
    return true
  })
}
