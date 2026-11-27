/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { createFileRoute, redirect, useLocation } from '@tanstack/react-router'

import { PluginsPage } from '@/features/plugins'
import {
  canOpenConfiguration,
  configurationOwner,
} from '@/features/plugins/configuration'
import { ROLE } from '@/lib/roles'
import { useAuthStore } from '@/stores/auth-store'

export const Route = createFileRoute('/_authenticated/extensions/plugins/')({
  validateSearch: (search): { configure?: string } => ({
    configure:
      typeof search.configure === 'string' &&
      configurationOwner(search.configure)
        ? search.configure
        : undefined,
  }),
  beforeLoad: ({ search }) => {
    const { auth } = useAuthStore.getState()
    if (
      !auth.session ||
      !auth.accessToken ||
      !auth.user ||
      auth.user.role < ROLE.ADMIN ||
      (search.configure &&
        !canOpenConfiguration(
          search.configure,
          auth.user.role === ROLE.SUPER_ADMIN
        ))
    ) {
      throw redirect({ to: '/403' })
    }
  },
  component: function PluginSettingsRoute() {
    const { configure } = Route.useSearch()
    const hash = useLocation({ select: (location) => location.hash })
    const navigate = Route.useNavigate()
    return (
      <PluginsPage
        panel={configure}
        paymentHash={hash}
        onPanelChange={(panel) => {
          void navigate({
            search: (previous) => ({ ...previous, configure: panel }),
            resetScroll: false,
          })
        }}
      />
    )
  },
})
