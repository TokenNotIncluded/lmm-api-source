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
import { useMemo } from 'react'
import { useTranslation } from 'react-i18next'

import type { NavGroup } from '@/components/layout/types'
import { useRuntimeCapabilities } from '@/context/runtime-capabilities-context'
import { ROLE } from '@/lib/roles'
import { applyRuntimeNavigation } from '@/lib/runtime-capabilities'
import { useAuthStore } from '@/stores/auth-store'

import { useSidebarConfig } from './use-sidebar-config'
import { useSidebarData } from './use-sidebar-data'

/**
 * Root navigation filtered by role, sidebar modules and runtime capabilities.
 * Route-level guards continue to enforce access independently.
 */
export function useRootSidebarNavigation(): NavGroup[] {
  const { t } = useTranslation()
  const userRole = useAuthStore((s) => s.auth.user?.role)
  const rootSidebarData = useSidebarData()
  const configFilteredRoot = useSidebarConfig(rootSidebarData.navGroups)
  const runtime = useRuntimeCapabilities()

  const rootNavGroups = useMemo<NavGroup[]>(() => {
    const role = userRole ?? ROLE.GUEST
    const isAdmin = role >= ROLE.ADMIN
    return configFilteredRoot
      .filter((group) => (group.id === 'admin' ? isAdmin : true))
      .map((group) => {
        const items = group.items.filter(
          (item) => item.requiredRole === undefined || role >= item.requiredRole
        )
        return items.length === group.items.length ? group : { ...group, items }
      })
  }, [configFilteredRoot, userRole])

  return useMemo(
    () =>
      runtime?.hasPluginInventoryAccess
        ? applyRuntimeNavigation(rootNavGroups, runtime.plugins, {
            unknown: t('Capability unknown'),
            unavailable: t('Capability unavailable'),
          })
        : rootNavGroups,
    [rootNavGroups, runtime, t]
  )
}
