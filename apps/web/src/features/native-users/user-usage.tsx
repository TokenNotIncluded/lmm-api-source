/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useQuery } from '@tanstack/react-query'
import { useEffect } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { toIntlLocale } from '@/i18n/languages'

import { NativeUsersFailure, type NativeUsersScope } from './access'
import { readNativeUser } from './api'

export function NativeUserUsage({
  scope,
  userId,
  onAccessDenied,
}: {
  scope: NativeUsersScope
  userId: string
  onAccessDenied: () => void
}) {
  const { t, i18n } = useTranslation()
  const query = useQuery({
    queryKey: [
      'native-user-usage',
      scope.userId,
      scope.sessionId,
      scope.role,
      userId,
    ],
    queryFn: ({ signal }) => readNativeUser(scope, userId, signal),
    retry: false,
    gcTime: 0,
    refetchOnWindowFocus: false,
  })
  const denied =
    query.error instanceof NativeUsersFailure &&
    (query.error.kind === 'forbidden' || query.error.kind === 'session_changed')
  useEffect(() => {
    if (denied) onAccessDenied()
  }, [denied, onAccessDenied])
  if (denied) return null
  if (query.isPending) {
    return (
      <Skeleton className='h-24 w-full' aria-label={t('Loading usage...')} />
    )
  }
  if (query.isError) {
    return (
      <div className='flex flex-wrap items-center gap-3 text-sm' role='status'>
        <p>{t('Account usage could not be loaded.')}</p>
        <Button
          variant='outline'
          size='sm'
          onClick={() => void query.refetch()}
        >
          {t('Retry')}
        </Button>
      </div>
    )
  }
  const locale = toIntlLocale(i18n.resolvedLanguage || i18n.language)
  const number = (value: string) =>
    new Intl.NumberFormat(locale).format(BigInt(value))
  const statistics = query.data.statistics
  const history = statistics?.historical_counters
  const usage = statistics?.usage
  const native = query.data.usage
  const values =
    usage?.status === 'available'
      ? [
          [t('Requests'), number(usage.requests)],
          [t('Tokens'), number(usage.tokens)],
          [
            t('Recorded consumption'),
            usage.credits === null
              ? t('Not priced')
              : t('{{amount}} credits', { amount: number(usage.credits) }),
          ],
          [t('Models'), number(usage.model_count)],
        ]
      : native.status === 'available'
        ? [
            [t('Settled requests'), number(native.settled_requests)],
            [t('Pending requests'), number(native.pending_requests)],
            [
              t('Recorded consumption'),
              t('{{amount}} credits', {
                amount: number(native.net_consumption_units),
              }),
            ],
          ]
        : []
  return (
    <section className='space-y-3' aria-label={t('Account usage')}>
      <div className='flex flex-wrap items-baseline justify-between gap-2'>
        <h3 className='text-sm font-medium'>{t('Account usage')}</h3>
        {usage?.status === 'available' && (
          <p className='text-muted-foreground text-xs'>
            {t('Current usage and imported history')}
          </p>
        )}
      </div>
      {values.length > 0 ? (
        <dl className='grid grid-cols-2 gap-4 xl:grid-cols-4'>
          {values.map(([label, value]) => (
            <div key={label} className='min-w-0'>
              <dt className='text-muted-foreground text-xs'>{label}</dt>
              <dd className='mt-1 text-base font-medium break-all tabular-nums'>
                {value}
              </dd>
            </div>
          ))}
        </dl>
      ) : (
        <p className='text-muted-foreground text-sm'>
          {t('Usage totals have not been reported for this account.')}
        </p>
      )}
      {usage?.status === 'available' ? (
        <p className='text-muted-foreground text-xs'>
          {t('Observed at {{time}}', {
            time: new Intl.DateTimeFormat(locale, {
              dateStyle: 'medium',
              timeStyle: 'short',
            }).format(usage.observed_at_unix_ms),
          })}
        </p>
      ) : native.status === 'available' ? (
        <p className='text-muted-foreground text-xs'>
          {t(
            'Native settled usage only. Imported counters are shown separately when available.'
          )}
        </p>
      ) : null}
      {history?.status === 'available' && (
        <details className='text-sm'>
          <summary className='text-muted-foreground min-h-11 cursor-pointer py-3'>
            {t('Original imported counters')}
          </summary>
          <dl className='flex flex-wrap gap-x-8 gap-y-3'>
            <div>
              <dt className='text-muted-foreground text-xs'>{t('Requests')}</dt>
              <dd className='mt-1 tabular-nums'>
                {number(history.request_count)}
              </dd>
            </div>
            <div>
              <dt className='text-muted-foreground text-xs'>
                {t('Raw credits')}
              </dt>
              <dd className='mt-1 tabular-nums'>
                {number(history.used_quota)}
              </dd>
            </div>
          </dl>
          <p className='text-muted-foreground mt-3 text-xs'>
            {t(
              'Imported counters are historical records, not spendable balance.'
            )}
          </p>
        </details>
      )}
    </section>
  )
}
