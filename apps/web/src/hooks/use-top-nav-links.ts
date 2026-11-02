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

import { useStatus } from '@/hooks/use-status'
import { getAuthenticatedLandingRoute } from '@/lib/console-activation'
import { parseHeaderNavModulesFromStatus } from '@/lib/nav-modules'
import {
  buildTopNavLinks,
  type TopNavLink,
  type TopNavSurface,
} from '@/lib/top-nav-policy'
import { useAuthStore } from '@/stores/auth-store'

export type { TopNavLink, TopNavSurface } from '@/lib/top-nav-policy'

export function useTopNavLinks(
  surface: TopNavSurface = 'default'
): TopNavLink[] {
  const { t } = useTranslation()
  const { status } = useStatus()
  const { auth } = useAuthStore()
  const modules = useMemo(
    () =>
      parseHeaderNavModulesFromStatus(status as Record<string, unknown> | null),
    [status]
  )
  return buildTopNavLinks({
    modules,
    surface,
    isAuthed: !!auth.user,
    workspaceHref: auth.user ? getAuthenticatedLandingRoute(auth.user) : '',
    hasDocsAccess: auth.user?.permissions?.docs_access === true,
    docsLink: status?.docs_link as string | undefined,
    translate: (key) => t(key),
  })
}
