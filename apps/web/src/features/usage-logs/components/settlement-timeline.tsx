/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useTranslation } from 'react-i18next'

import type { UsageLog } from '../data/schema'

export function SettlementTimeline({
  timeline,
  state,
}: {
  timeline: NonNullable<UsageLog['settlement_timeline']>
  state?: string
}) {
  const { t, i18n } = useTranslation()
  const date = (timestamp: number) =>
    new Date(timestamp * 1000).toLocaleString(i18n.language)
  const labels = {
    reserve_total: t('Total reserved after this operation'),
    captured: t('Usage captured'),
    released: t('Reservation released'),
    refunded: t('Refund'),
  }
  return (
    <section className='space-y-2 text-sm'>
      <h3 className='font-medium'>{t('Settlement timeline')}</h3>
      {timeline.events.length ? (
        <ol className='space-y-1'>
          {timeline.events.map((event, index) => (
            <li key={index} className='grid grid-cols-[1fr_auto] gap-x-3'>
              <span>{labels[event.kind]}</span>
              <span className='tabular-nums'>
                {t('{{amount}} credits', {
                  amount: event.credits.toLocaleString(i18n.language),
                })}
              </span>
              <time
                className='text-muted-foreground text-xs'
                dateTime={new Date(event.at * 1000).toISOString()}
              >
                {date(event.at)}
              </time>
            </li>
          ))}
        </ol>
      ) : (
        <p>{t('No charging command is recorded.')}</p>
      )}
      {timeline.truncated && (
        <p>{t('Showing the latest 200 charging commands.')}</p>
      )}
      {timeline.wait_seconds_at_read !== null && (
        <div className='text-muted-foreground space-y-1'>
          <p>
            {t('Time since authorization at last refresh: {{seconds}} s', {
              seconds: timeline.wait_seconds_at_read.toLocaleString(
                i18n.language
              ),
            })}
          </p>
          <p>
            {timeline.hold_retained_at === null
              ? t('No reconciliation result is recorded.')
              : t('Last reconciliation: hold retained at {{time}}.', {
                  time: date(timeline.hold_retained_at),
                })}
          </p>
          <p>
            {state === 'reconcile'
              ? t(
                  'The hold stays in place until provider evidence supports settlement.'
                )
              : state === 'reserved'
                ? t(
                    'If processing never starts, the expired reservation can be released.'
                  )
                : t(
                    'Core is waiting for final usage. An uncertain outcome keeps the reservation for reconciliation.'
                  )}
          </p>
        </div>
      )}
    </section>
  )
}
