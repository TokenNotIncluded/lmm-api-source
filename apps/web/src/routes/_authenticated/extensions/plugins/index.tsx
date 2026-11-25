/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { createFileRoute, redirect } from '@tanstack/react-router'

import { PluginsPage } from '@/features/plugins'
import { ROLE } from '@/lib/roles'
import { useAuthStore } from '@/stores/auth-store'

export const Route = createFileRoute('/_authenticated/extensions/plugins/')({
  validateSearch: (
    search: Record<string, unknown>
  ): { configure?: string; section?: string } => ({
    configure:
      typeof search.configure === 'string' ? search.configure : undefined,
    section: typeof search.section === 'string' ? search.section : undefined,
  }),
  beforeLoad: () => {
    const { auth } = useAuthStore.getState()
    if (!auth.user || auth.user.role < ROLE.ADMIN) {
      throw redirect({ to: '/403' })
    }
  },
  component: PluginRoute,
})

function PluginRoute() {
  const { configure, section } = Route.useSearch()
  return <PluginsPage configure={configure} section={section} />
}
