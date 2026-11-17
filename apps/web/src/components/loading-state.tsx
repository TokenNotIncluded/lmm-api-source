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
import { Loader2 } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { LmmBrandMark } from '@/components/lmm-brand-mark'
import { cn } from '@/lib/utils'

import { WaitCompanion } from './wait-companion'

import './loading-state.css'

interface LoadingStateProps {
  className?: string
  message?: string
  size?: 'sm' | 'md' | 'lg'
  inline?: boolean
}

const sizeMap = {
  sm: 'size-4',
  md: 'size-6',
  lg: 'size-8',
} as const

/** Route and application startup share a full-height, non-suspending shell. */
export function PageLoadingState() {
  const { t } = useTranslation(undefined, { useSuspense: false })

  return (
    <main className='page-loading' aria-busy='true'>
      <div className='page-loading-content' role='status' aria-live='polite'>
        <div className='page-loading-identity' aria-hidden='true'>
          <LmmBrandMark className='size-10' />
          <span>LMM API</span>
        </div>
        <div className='page-loading-track' aria-hidden='true'>
          <span />
        </div>
        <p>{t('Preparing your workspace')}</p>
      </div>
    </main>
  )
}

export function LoadingState(props: LoadingStateProps) {
  const { t } = useTranslation()
  const iconSize = sizeMap[props.size ?? 'md']

  if (props.inline) {
    return (
      <span
        className={cn('inline-flex items-center gap-2', props.className)}
        role='status'
        aria-label={props.message ?? t('Loading...')}
      >
        <Loader2
          className={cn(iconSize, 'animate-spin motion-reduce:animate-none')}
          aria-hidden='true'
        />
        {props.message != null && (
          <span className='text-muted-foreground text-sm'>{props.message}</span>
        )}
      </span>
    )
  }

  return (
    <div
      className={cn(
        'flex min-h-[200px] flex-col items-center justify-center gap-3',
        props.className
      )}
      role='status'
      aria-live='polite'
      aria-busy='true'
    >
      <div
        className='text-primary animate-spin motion-reduce:animate-none'
        aria-hidden='true'
      >
        <Loader2 className={iconSize} />
      </div>
      <p className='text-muted-foreground text-sm'>
        {props.message ?? t('Loading...')}
      </p>
      <WaitCompanion pending className='items-center' />
    </div>
  )
}
