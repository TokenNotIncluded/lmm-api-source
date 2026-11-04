/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useQuery } from '@tanstack/react-query'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { TitledCard } from '@/components/ui/titled-card'
import { toIntlLocale } from '@/i18n/languages'
import { useAuthStore } from '@/stores/auth-store'

import { readSmsHistory, type SmsHistoryScope } from './sms-history'

export function SmsHistoryCard() {
  const userId = useAuthStore((s) => s.auth.user?.id)
  const sessionId = useAuthStore((s) => s.auth.session?.sid)
  if (!userId || !sessionId) return null
  return (
    <History
      key={JSON.stringify([userId, sessionId])}
      scope={{ userId, sessionId }}
    />
  )
}
function History({ scope }: { scope: SmsHistoryScope }) {
  const { t, i18n } = useTranslation()
  const locale = toIntlLocale(i18n.resolvedLanguage || i18n.language)
  const [page, setPage] = useState(1)
  const query = useQuery({
    queryKey: ['archived-sms-history', scope.userId, scope.sessionId, page],
    queryFn: ({ signal }) => readSmsHistory(scope, page, signal),
    retry: false,
    gcTime: 0,
  })
  const data = query.data
  const date = (seconds: number) =>
    new Date(seconds * 1000).toLocaleString(locale)
  const units = (raw: string) => BigInt(raw).toLocaleString(locale)
  return (
    <TitledCard
      title={t('Archived SMS financial history')}
      description={t(
        'Frozen history only. Original quota units are not your current balance; viewing does not order, refund, or contact the supplier.'
      )}
    >
      <div className='space-y-4'>
        {query.isFetching ? (
          <p role='status'>{t('Loading...')}</p>
        ) : query.isError ? (
          <div role='alert' className='space-y-2'>
            <p>{t('Archived SMS history is unavailable.')}</p>
            <Button variant='outline' onClick={() => void query.refetch()}>
              {t('Retry')}
            </Button>
          </div>
        ) : data ? (
          <>
            <p className='text-muted-foreground text-sm'>
              {t('Source snapshot: {{date}}', { date: date(data.observed_at) })}
            </p>
            {data.items.length === 0 ? (
              <p>{t('No visible archived SMS orders.')}</p>
            ) : (
              <ul className='divide-y'>
                {data.items.map((order) => (
                  <li key={order.id} className='space-y-3 py-4 text-sm'>
                    <div className='flex flex-wrap justify-between gap-2'>
                      <span className='min-w-0 font-medium break-all'>
                        {order.id} · {order.service}
                      </span>
                      <span>
                        {t(
                          order.status === 'completed'
                            ? 'Completed'
                            : order.status === 'cancelled'
                              ? 'Cancelled'
                              : 'Failed'
                        )}
                      </span>
                    </div>
                    <time
                      dateTime={new Date(order.created_at * 1000).toISOString()}
                    >
                      {date(order.created_at)}
                    </time>
                    <dl className='grid gap-3 sm:grid-cols-2 lg:grid-cols-4'>
                      <div>
                        <dt className='text-muted-foreground'>
                          {t('Original reserved units')}
                        </dt>
                        <dd className='break-all tabular-nums'>
                          {units(order.raw_reserved_units)}
                        </dd>
                      </div>
                      <div>
                        <dt className='text-muted-foreground'>
                          {t('Original refunded units')}
                        </dt>
                        <dd className='break-all tabular-nums'>
                          {units(order.raw_refunded_units)}
                        </dd>
                      </div>
                      <div>
                        <dt className='text-muted-foreground'>
                          {t('Original net debit units')}
                        </dt>
                        <dd className='break-all tabular-nums'>
                          {units(order.raw_net_debit_units)}
                        </dd>
                      </div>
                      <div>
                        <dt className='text-muted-foreground'>
                          {t('Quoted price (USD)')}
                        </dt>
                        <dd className='break-all tabular-nums'>
                          {order.quoted_price_usd ?? t('Unknown')}
                        </dd>
                      </div>
                    </dl>
                    <details>
                      <summary className='cursor-pointer'>
                        {t('Ledger entries')}
                      </summary>
                      <ul className='mt-2 space-y-1'>
                        {order.ledger.map((entry, index) => (
                          <li
                            key={index}
                            className='flex flex-wrap justify-between gap-2'
                          >
                            <span>
                              {date(entry.created_at)} ·{' '}
                              {t(
                                entry.entry_type === 'reserve'
                                  ? 'Reserved'
                                  : 'Refunded'
                              )}
                            </span>
                            <span className='break-all tabular-nums'>
                              {units(entry.raw_amount_units)}
                            </span>
                          </li>
                        ))}
                      </ul>
                    </details>
                  </li>
                ))}
              </ul>
            )}
            <div className='flex flex-wrap items-center justify-between gap-2'>
              <Button
                variant='outline'
                disabled={page === 1}
                onClick={() => setPage(page - 1)}
              >
                {t('Previous page')}
              </Button>
              <span className='text-sm tabular-nums'>
                {page} / {Math.max(1, Math.ceil(data.total / data.page_size))}
              </span>
              <Button
                variant='outline'
                disabled={
                  page * data.page_size >= data.total || page === 100_000
                }
                onClick={() => setPage(page + 1)}
              >
                {t('Next page')}
              </Button>
            </div>
          </>
        ) : null}
      </div>
    </TitledCard>
  )
}
