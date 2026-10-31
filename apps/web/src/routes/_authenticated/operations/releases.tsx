/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { createFileRoute, redirect } from '@tanstack/react-router'

import { ReleaseNotesAdmin } from '@/features/release-notes/admin-page'
import { canManageReleaseNotes } from '@/features/release-notes/api'
import { useAuthStore } from '@/stores/auth-store'

export const Route = createFileRoute('/_authenticated/operations/releases')({
  beforeLoad: () => {
    if (!canManageReleaseNotes(useAuthStore.getState().auth.user)) {
      throw redirect({ to: '/403' })
    }
  },
  component: ReleaseNotesAdmin,
})
