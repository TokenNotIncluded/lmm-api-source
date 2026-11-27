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
import {
  Box,
  CreditCard,
  Layout,
  Settings,
  Shield,
  ShieldAlert,
  Wrench,
} from 'lucide-react'

import { legacyPluginPanel } from '@/features/plugins/configuration'
import { getAuthSectionNavItems } from '@/features/system-settings/auth/section-registry.tsx'
import { getBillingSectionNavItems } from '@/features/system-settings/billing/section-registry.tsx'
import { getContentSectionNavItems } from '@/features/system-settings/content/section-registry.tsx'
import { getModelsSectionNavItems } from '@/features/system-settings/models/section-registry.tsx'
import { getOperationsSectionNavItems } from '@/features/system-settings/operations/section-registry.tsx'
import { getSecuritySectionNavItems } from '@/features/system-settings/security/section-registry.tsx'
import { getSiteSectionNavItems } from '@/features/system-settings/site/section-registry.tsx'
import { ROLE } from '@/lib/roles'
import {
  canReadSystemSettingsPath,
  getSettingsScope,
  getSettingsTitle,
  type SettingsScope,
} from '@/lib/system-settings-access'

import type { NavGroup } from '../types'

/** Settings links for the page search and settings landing pages. */
export function getSystemSettingsNavGroups(
  t: TFunction,
  role: number = ROLE.SUPER_ADMIN,
  scope?: SettingsScope,
  includePluginLinks = false
): NavGroup[] {
  const groups: NavGroup[] = [
    {
      id: 'system-administration',
      title: t(scope ? getSettingsTitle(scope) : 'System Administration'),
      items: [
        {
          title: t('Site & Branding'),
          icon: Settings,
          items: getSiteSectionNavItems(t),
        },
        {
          title: t(scope === 'core' ? 'Identity' : 'Authentication'),
          icon: Shield,
          items: [
            ...(scope === 'core' ? [{ title: t('Users'), url: '/users' }] : []),
            ...getAuthSectionNavItems(t),
          ],
        },
        {
          title: t('Billing & Payment'),
          icon: CreditCard,
          items: getBillingSectionNavItems(t),
        },
        {
          title: t('Models & Routing'),
          icon: Box,
          items: getModelsSectionNavItems(t).filter(
            (item) =>
              item.url !== '/system-settings/models/service-tier-pricing'
          ),
        },
        {
          title: t(scope === 'core' ? 'API Access' : 'Security & Limits'),
          icon: ShieldAlert,
          items: [
            ...(scope === 'core'
              ? [{ title: t('API Keys'), url: '/keys' }]
              : []),
            ...getSecuritySectionNavItems(t),
          ],
        },
        {
          title: t('Console Content'),
          icon: Layout,
          items: getContentSectionNavItems(t),
        },
        {
          title: t('Operations'),
          icon: Wrench,
          items: getOperationsSectionNavItems(t),
        },
      ],
    },
  ]
  if (role === ROLE.SUPER_ADMIN && !scope) return groups
  return groups
    .map((group) => ({
      ...group,
      items: group.items.flatMap((section) => {
        if (!section.items) return []
        const items = section.items.filter(
          (entry) =>
            entry.url &&
            canReadSystemSettingsPath(role, entry.url) &&
            (!scope || includePluginLinks || !legacyPluginPanel(entry.url)) &&
            (!scope || getSettingsScope(entry.url) === scope)
        )
        return items.length ? [{ ...section, items }] : []
      }),
    }))
    .filter((group) => group.items.length)
}
