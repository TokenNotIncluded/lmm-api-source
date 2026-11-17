/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { AlertCircle, CircleCheck, CircleX } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Skeleton } from '@/components/ui/skeleton'
import { toIntlLocale } from '@/i18n/languages'
import { cn } from '@/lib/utils'

import type { ServiceHealth } from './api'

export function ServiceHealthPanel(props: {
  health?: ServiceHealth
  error: unknown
  loading: boolean
  checkedAt: number
}) {
  const { t, i18n } = useTranslation()
  const rows = props.health
    ? [
        { label: t('API service'), ready: props.health.status === 'ready' },
        { label: t('Database'), ready: props.health.database },
        {
          label: t('Model routing'),
          ready:
            props.health.status === 'ready' && props.health.capabilities.models,
        },
      ]
    : []

  return (
    <section aria-labelledby='service-health-heading' className='space-y-4'>
      <div className='flex flex-wrap items-baseline justify-between gap-2'>
        <h2 id='service-health-heading' className='text-base font-semibold'>
          {t('Current service status')}
        </h2>
        {props.checkedAt > 0 && (
          <p className='text-muted-foreground text-xs'>
            {t('Last checked: {{time}}', {
              time: new Date(props.checkedAt).toLocaleString(
                toIntlLocale(i18n.resolvedLanguage || i18n.language)
              ),
            })}
          </p>
        )}
      </div>
      {props.loading ? (
        <Skeleton className='h-24 rounded-xl' />
      ) : props.error ? (
        <Alert variant='destructive'>
          <AlertCircle aria-hidden='true' />
          <AlertTitle>{t('Service check failed')}</AlertTitle>
          <AlertDescription>
            {t(
              'The health endpoint did not return a valid response. Refresh to check again.'
            )}
          </AlertDescription>
        </Alert>
      ) : (
        <dl className='bg-muted/35 grid gap-4 rounded-xl p-5 sm:grid-cols-3'>
          {rows.map((row) => {
            const Icon = row.ready ? CircleCheck : CircleX
            return (
              <div key={row.label} className='min-w-0'>
                <dt className='text-muted-foreground text-sm'>{row.label}</dt>
                <dd
                  className={cn(
                    'mt-2 flex items-center gap-2 font-medium',
                    row.ready ? 'text-success' : 'text-warning'
                  )}
                >
                  <Icon className='size-4 shrink-0' aria-hidden='true' />
                  {row.ready ? t('Ready') : t('Unavailable')}
                </dd>
              </div>
            )
          })}
        </dl>
      )}
      <p className='text-muted-foreground max-w-3xl text-xs leading-relaxed'>
        {t(
          'Service readiness checks the API and database. Provider performance comes from actual requests below.'
        )}
      </p>
    </section>
  )
}
