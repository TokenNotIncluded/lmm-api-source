/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import { Link } from '@tanstack/react-router'
import { ArrowLeft } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { cn } from '@/lib/utils'

export function BackToConsole({ className }: { className?: string }) {
  const { t } = useTranslation()
  return (
    <Link
      to='/dashboard/$section'
      params={{ section: 'overview' }}
      referrerPolicy='no-referrer'
      className={cn(
        'text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:ring-ring inline-flex min-h-11 min-w-0 items-center gap-2 rounded-md max-sm:gap-1 px-2 text-sm font-medium focus-visible:ring-2 focus-visible:outline-none',
        className
      )}
    >
      <ArrowLeft className='size-4 shrink-0' aria-hidden='true' />
      <span className='min-w-0 text-start leading-tight [overflow-wrap:anywhere] whitespace-normal'>
        {t('Back to Dashboard')}
      </span>
    </Link>
  )
}
