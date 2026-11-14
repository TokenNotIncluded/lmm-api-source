/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { Pause, RefreshCw, X } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { toIntlLocale } from '@/i18n/languages'
import { cn } from '@/lib/utils'

export function CommonLogsStatus(props: {
  total: number | undefined
  updatedAt: number
  refreshing: boolean
  automatic: boolean
  canAutoRefresh: boolean
  onAutomaticChange: (enabled: boolean) => void
  onRefresh: () => void
  onCancel: () => void
}) {
  const { t, i18n } = useTranslation()
  const locale = toIntlLocale(i18n.resolvedLanguage || i18n.language)
  return (
    <div className='flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1 text-xs'>
      <span className='text-muted-foreground tabular-nums'>
        {t('Total')}:{' '}
        <span className='text-foreground'>
          {props.total?.toLocaleString(locale) ?? '—'}
        </span>
      </span>
      {props.updatedAt > 0 && (
        <time
          className='text-muted-foreground tabular-nums'
          dateTime={new Date(props.updatedAt).toISOString()}
          title={t('Last updated')}
        >
          {new Date(props.updatedAt).toLocaleTimeString(locale)}
        </time>
      )}
      <Button
        type='button'
        variant={props.automatic ? 'secondary' : 'ghost'}
        size='sm'
        aria-pressed={props.automatic}
        disabled={!props.canAutoRefresh}
        title={
          props.canAutoRefresh
            ? t(
                'Refreshes every 5 seconds within the selected range. Pauses in the background.'
              )
            : t(
                'Auto refresh is available on the first page of current requests.'
              )
        }
        onClick={() => props.onAutomaticChange(!props.automatic)}
      >
        {props.automatic && <Pause className='size-3.5' aria-hidden='true' />}
        {t('Auto refresh')}
      </Button>
      <Button
        type='button'
        variant='ghost'
        size='sm'
        onClick={props.refreshing ? props.onCancel : props.onRefresh}
      >
        {props.refreshing ? (
          <X className='size-3.5' aria-hidden='true' />
        ) : (
          <RefreshCw className={cn('size-3.5')} aria-hidden='true' />
        )}
        {props.refreshing ? t('Cancel') : t('Refresh')}
      </Button>
    </div>
  )
}
