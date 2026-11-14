/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { Unlink } from 'lucide-react'
import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import { StatusBadge } from '@/components/status-badge'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'

export function BindingCard({
  label,
  value,
  icon,
  bound,
  showBadge = true,
  action,
}: {
  label: string
  value?: string
  icon: ReactNode
  bound: boolean
  showBadge?: boolean
  action?: {
    label: string
    onClick: () => void
    disabled?: boolean
    destructive?: boolean
    variant?: 'ghost' | 'outline' | 'destructive'
  }
}) {
  const { t } = useTranslation()
  return (
    <div className='flex items-center justify-between gap-2.5 rounded-lg border p-2.5 sm:gap-3 sm:p-3'>
      <div className='flex min-w-0 items-center gap-2.5 sm:gap-3'>
        <div className='bg-muted shrink-0 rounded-md p-1.5 sm:p-2'>{icon}</div>
        <div className='min-w-0'>
          <div className='flex items-center gap-1.5'>
            <p className='text-sm font-medium'>{label}</p>
            {bound && showBadge && (
              <StatusBadge
                label={t('Bound')}
                variant='success'
                copyable={false}
              />
            )}
          </div>
          <p className='text-muted-foreground truncate text-xs' title={value}>
            {value || t('Not bound')}
          </p>
        </div>
      </div>
      {action && (
        <Button
          variant={
            action.variant ?? (action.destructive ? 'destructive' : 'outline')
          }
          size='sm'
          className={cn(
            'h-7 shrink-0 px-2.5 text-xs',
            action.variant === 'ghost' && 'text-destructive'
          )}
          disabled={action.disabled}
          onClick={action.onClick}
        >
          {action.destructive && <Unlink data-icon='inline-start' />}
          {action.label}
        </Button>
      )}
    </div>
  )
}
