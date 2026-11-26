/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { Boxes, Server } from 'lucide-react'

import { getLobeIcon } from '@/lib/lobe-icon'

export function NativeProviderLogo({
  icon,
  model = false,
}: {
  icon?: string
  model?: boolean
}) {
  const Fallback = model ? Boxes : Server
  return (
    <span
      className='bg-muted flex size-7 shrink-0 items-center justify-center rounded-md'
      aria-hidden='true'
    >
      {icon ? getLobeIcon(icon, 17) : <Fallback className='size-4' />}
    </span>
  )
}
