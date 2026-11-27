/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { createFileRoute, redirect } from '@tanstack/react-router'

import { ManagementTerminal } from '@/features/admin-terminal/page'
import { readNativeRoutingScope } from '@/features/channels/native-api'

export const Route = createFileRoute('/_authenticated/operations/terminal')({
  beforeLoad: () => {
    if (!readNativeRoutingScope()) throw redirect({ to: '/403' })
  },
  component: ManagementTerminal,
})
