/*
Copyright (C) 2023-2026 QuantumNous

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

For commercial licensing, please contact support@quantumnous.com
*/
import type { TFunction } from 'i18next'
import { Box, Settings } from 'lucide-react'

import { getAuthSectionNavItems } from '@/features/system-settings/auth/section-registry'
import { getBillingSectionNavItems } from '@/features/system-settings/billing/section-registry'
import { getContentSectionNavItems } from '@/features/system-settings/content/section-registry'
import { legacySettingsDestination } from '@/features/system-settings/legacy-settings-destination'
import { getModelsSectionNavItems } from '@/features/system-settings/models/section-registry'
import { getOperationsSectionNavItems } from '@/features/system-settings/operations/section-registry'
import { getSecuritySectionNavItems } from '@/features/system-settings/security/section-registry'
import { getSiteSectionNavItems } from '@/features/system-settings/site/section-registry'
import { ROLE } from '@/lib/roles'
import { useAuthStore } from '@/stores/auth-store'

import type { NavGroup, SidebarView } from '../types'

export function getSystemSettingsNavGroups(
  t: TFunction,
  role: number = ROLE.SUPER_ADMIN
): NavGroup[] {
  if (role < ROLE.ADMIN) return []
  const groups: NavGroup[] = [
    {
      id: 'system-administration',
      title: t('System Administration'),
      items: [
        ...(role === ROLE.SUPER_ADMIN
          ? [
              {
                title: t('System Settings'),
                url: '/system-settings/site',
                icon: Settings,
              },
            ]
          : []),
        {
          title: t('Plugin management'),
          url: '/extensions/plugins',
          icon: Box,
        },
      ],
    },
  ]
  if (role !== ROLE.SUPER_ADMIN) return groups

  const categories = [
    ['site', t('Site & Branding'), getSiteSectionNavItems],
    ['auth', t('Authentication'), getAuthSectionNavItems],
    ['billing', t('Billing & Payment'), getBillingSectionNavItems],
    ['models', t('Models & Routing'), getModelsSectionNavItems],
    ['security', t('Security & Limits'), getSecuritySectionNavItems],
    ['content', t('Console Content'), getContentSectionNavItems],
    ['operations', t('Operations'), getOperationsSectionNavItems],
  ] as const
  for (const [category, title, getItems] of categories) {
    const items = getItems(t).filter(({ url }) => {
      const section = url.slice(url.lastIndexOf('/') + 1)
      const destination = legacySettingsDestination(category, section)
      return destination && destination.action !== 'unavailable'
    })
    if (items.length) groups.push({ id: category, title, items })
  }
  return groups
}

export const SYSTEM_SETTINGS_VIEW: SidebarView = {
  id: 'system-settings',
  pathPattern: /^\/system-settings(\/|$)/,
  parent: { to: '/dashboard/overview', label: 'Back to Dashboard' },
  getNavGroups: (t) =>
    getSystemSettingsNavGroups(
      t,
      useAuthStore.getState().auth.user?.role ?? ROLE.GUEST
    ),
}
