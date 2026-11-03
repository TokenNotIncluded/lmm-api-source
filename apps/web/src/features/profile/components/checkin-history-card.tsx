/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useQuery } from '@tanstack/react-query'
import { useId, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { TitledCard } from '@/components/ui/titled-card'
import { toIntlLocale } from '@/i18n/languages'
import { useAuthStore } from '@/stores/auth-store'

import {
  readCheckinHistory,
  type CheckinHistoryScope,
} from '../checkin-history'

export function CheckinHistoryCard() {
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

function History({ scope }: { scope: CheckinHistoryScope }) {
  const { t, i18n } = useTranslation()
  const locale = toIntlLocale(i18n.resolvedLanguage || i18n.language)
  const id = useId()
  const [month, setMonth] = useState(() => {
    const now = new Date()
    return `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, '0')}`
  })
  const query = useQuery({
    queryKey: ['checkin-history', scope.userId, scope.sessionId, month],
    queryFn: ({ signal }) => readCheckinHistory(scope, month, signal),
    retry: false,
    gcTime: 0,
  })
  const data = query.data
  return (
    <TitledCard
      title={t('Check-in history')}
      description={t(
        'Original reward units, not your current balance. Viewing history does not issue rewards.'
      )}
    >
      <div className='space-y-4'>
        <div className='space-y-2'>
          <label htmlFor={id} className='text-sm font-medium'>
            {t('Month')}
          </label>
          <Input
            id={id}
            type='month'
            min='0001-01'
            max='9999-12'
            value={month}
            onChange={(e) => {
              if (/^(?!0000)\d{4}-(0[1-9]|1[0-2])$/.test(e.target.value)) {
                setMonth(e.target.value)
              }
            }}
          />
        </div>
        {query.isFetching ? (
          <p role='status'>{t('Loading...')}</p>
        ) : query.isError ? (
          <div role='alert' className='space-y-2'>
            <p>{t('Check-in history is unavailable.')}</p>
            <Button variant='outline' onClick={() => void query.refetch()}>
              {t('Retry')}
            </Button>
          </div>
        ) : data ? (
          <>
            <dl className='grid gap-3 text-sm sm:grid-cols-2'>
              <div>
                <dt className='text-muted-foreground'>
                  {t('Total check-ins')}
                </dt>
                <dd className='font-medium tabular-nums'>
                  {data.total_checkins.toLocaleString(locale)}
                </dd>
              </div>
              <div>
                <dt className='text-muted-foreground'>
                  {t('Original reward total')}
                </dt>
                <dd className='font-medium break-all tabular-nums'>
                  {BigInt(data.total_raw_quota_awarded).toLocaleString(locale)}
                </dd>
              </div>
            </dl>
            {data.records.length === 0 ? (
              <p>{t('No check-ins in this month.')}</p>
            ) : (
              <ul className='divide-y text-sm'>
                {data.records.map((record) => (
                  <li
                    key={record.checkin_date}
                    className='flex flex-wrap justify-between gap-2 py-2 tabular-nums'
                  >
                    <time dateTime={record.checkin_date}>
                      {record.checkin_date}
                    </time>
                    <span>
                      {record.raw_quota_awarded.toLocaleString(locale)}
                    </span>
                  </li>
                ))}
              </ul>
            )}
          </>
        ) : null}
      </div>
    </TitledCard>
  )
}
