/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { createFileRoute, redirect } from '@tanstack/react-router'

import { ROLE } from '@/lib/roles'
import { useAuthStore } from '@/stores/auth-store'

export const Route = createFileRoute('/_authenticated/extensions/settings/')({
  beforeLoad: () => {
    const { auth } = useAuthStore.getState()
    if (
      !auth.session ||
      !auth.accessToken ||
      (auth.user?.role ?? ROLE.GUEST) < ROLE.ADMIN
    ) {
      throw redirect({ to: '/403' })
    }
    throw redirect({ to: '/extensions/plugins', replace: true })
  },
})
