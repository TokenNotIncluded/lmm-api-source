/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useTranslation } from 'react-i18next'

import type { UsageLog } from '../data/schema'
import { failureFacts } from '../lib/failure-facts'
import {
  nativeRequestStates,
  requestResultLabels,
  settlementStatusLabels,
  usageLogStatusCode,
} from '../lib/receipt-state'
import type { LogOtherData } from '../types'

export function RequestFailureFacts({
  log,
  other,
  isAdmin,
}: {
  log: UsageLog
  other: LogOtherData | null
  isAdmin: boolean
}) {
  const { t } = useTranslation()
  const states = nativeRequestStates(log, other)
  const failed = states
    ? states.result === 'error' || states.result === 'disconnected'
    : log.type === 5
  if (!failed) return null
  const facts = failureFacts(log, other, isAdmin)
  const unknown = t('Not recorded')
  const rows = [
    [t('Status Code'), usageLogStatusCode(log, other)],
    [t('Error code'), facts.code],
    [
      t('Error message'),
      facts.message ??
        (facts.messageWithheld ? t('Message withheld') : undefined),
    ],
    [t('Protocol'), facts.protocol],
    [t('Request path'), facts.path],
    [t('Route failure'), facts.failureKind],
    [t('Retry or stop reason'), facts.endReason],
    ...(facts.observedRetry
      ? [[t('Observed retry'), facts.observedRetry]]
      : []),
  ]
  return (
    <section className='space-y-3'>
      <h3 className='font-semibold'>{t('Upstream error')}</h3>
      <dl className='grid grid-cols-2 gap-2 text-sm'>
        {rows.map(([label, value]) => (
          <div key={String(label)}>
            <dt className='text-muted-foreground'>{label}</dt>
            <dd className='break-words'>{value ?? unknown}</dd>
          </div>
        ))}
      </dl>
      {states && (
        <p className='text-sm'>
          {t('Request result')} ·{' '}
          {t(
            states.result ? requestResultLabels[states.result] : 'Not recorded'
          )}
          {' · '}
          {t('Settlement status')} ·{' '}
          {t(
            states.settlement
              ? settlementStatusLabels[states.settlement]
              : 'Not recorded'
          )}
        </p>
      )}
      {states && isAdmin && log.channel != null && log.channel > 0 && (
        <a
          className='text-sm underline'
          href={
            '/usage-logs/common?' +
            new URLSearchParams({
              source: 'native',
              scope: 'all',
              channel: String(log.channel),
              requestResult: 'error',
              startTime: String(Date.now() - 86_400_000),
              endTime: String(Date.now()),
            })
          }
        >
          {t('Recent errors on this channel')}
        </a>
      )}
    </section>
  )
}
