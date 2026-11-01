/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { isAxiosError } from 'axios'
import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Dialog } from '@/components/dialog'
import { LoadingState } from '@/components/loading-state'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { toIntlLocale } from '@/i18n/languages'
import { formatExactQuotaInCurrency } from '@/lib/currency'
import { formatTimestampToDate } from '@/lib/format'
import { DEFAULT_CURRENCY_CONFIG } from '@/stores/system-config-store'

import {
  getNativeChannelUsage,
  getNativeChannelCounters,
  type NativeChannelCounters,
  type NativeChannelUsage,
  type NativeRoutingScope,
} from '../../native-api'

type UsageState =
  | { status: 'loading' }
  | { status: 'ready'; usage: NativeChannelUsage }
  | {
      status: 'error'
      message:
        | 'channelUsage.identity'
        | 'channelUsage.denied'
        | 'channelUsage.error'
    }

export function NativeChannelUsageDialog({
  scope,
  channelId,
  channelName,
  countersEnabled,
  onClose,
}: {
  scope: NativeRoutingScope
  channelId: number
  channelName: string
  countersEnabled: boolean
  onClose: () => void
}) {
  const { t, i18n } = useTranslation()
  const [state, setState] = useState<UsageState>({ status: 'loading' })
  const [counters, setCounters] = useState<
    | { status: 'loading' }
    | { status: 'ready'; data: NativeChannelCounters }
    | {
        status: 'error'
        message:
          | 'channelCounters.error'
          | 'channelUsage.identity'
          | 'channelUsage.denied'
      }
  >({ status: 'loading' })
  const { userId, sessionId, role } = scope
  const locale = toIntlLocale(i18n.resolvedLanguage || i18n.language)
  useEffect(() => {
    const controller = new AbortController()
    let active = true
    setState({ status: 'loading' })
    setCounters({ status: 'loading' })
    void getNativeChannelUsage(
      { userId, sessionId, role },
      channelId,
      controller.signal
    ).then(
      (usage) => {
        if (active) setState({ status: 'ready', usage })
      },
      (error: unknown) => {
        if (!active) return
        const status = isAxiosError(error) ? error.response?.status : undefined
        setState({
          status: 'error',
          message:
            status === 409
              ? 'channelUsage.identity'
              : status === 403
                ? 'channelUsage.denied'
                : 'channelUsage.error',
        })
      }
    )
    if (countersEnabled) {
      void getNativeChannelCounters(
        { userId, sessionId, role },
        channelId,
        controller.signal
      ).then(
        (data) => {
          if (active) setCounters({ status: 'ready', data })
        },
        (error: unknown) => {
          if (active) {
            const status = isAxiosError(error)
              ? error.response?.status
              : undefined
            setCounters({
              status: 'error',
              message:
                status === 409
                  ? 'channelUsage.identity'
                  : status === 403
                    ? 'channelUsage.denied'
                    : 'channelCounters.error',
            })
          }
        }
      )
    }
    return () => {
      active = false
      controller.abort()
    }
  }, [userId, sessionId, role, channelId, countersEnabled])

  const amount = (
    value: string,
    usage: Pick<NativeChannelUsage, 'credits_per_usd'>
  ) =>
    formatExactQuotaInCurrency(
      value,
      'CREDIT',
      { locale, creditLabel: t('Credits') },
      {
        ...DEFAULT_CURRENCY_CONFIG,
        currencyUnit: 'credit',
        creditsPerUsd: 500000,
        creditsPerUsdExact: usage.credits_per_usd,
      }
    )
  const count = (value: string) =>
    new Intl.NumberFormat(locale).format(BigInt(value))

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
      title={t('channelUsage.title')}
      description={channelName}
      contentHeight='auto'
      footer={
        <Button variant='outline' onClick={onClose}>
          {t('Close')}
        </Button>
      }
    >
      <p className='text-muted-foreground mb-4 text-sm'>
        {t('channelUsage.note')}
      </p>
      {state.status === 'loading' && (
        <LoadingState inline message={t('Loading...')} />
      )}
      {state.status === 'error' && (
        <Alert variant='destructive' role='alert'>
          <AlertDescription>{t(state.message)}</AlertDescription>
        </Alert>
      )}
      {state.status === 'ready' && (
        <>
          <p className='text-muted-foreground mb-4 text-xs'>
            <time>{formatTimestampToDate(state.usage.start_timestamp)}</time>
            {' – '}
            <time>{formatTimestampToDate(state.usage.end_timestamp)}</time>
          </p>
          <dl className='space-y-3 text-sm'>
            {[
              [
                t('channelUsage.gross'),
                amount(state.usage.gross_settled_credits, state.usage),
              ],
              [t('Refunds'), amount(state.usage.refunded_credits, state.usage)],
              [
                t('Net consumption'),
                amount(state.usage.net_used_credits, state.usage),
              ],
              [t('channelUsage.settled'), count(state.usage.settled_requests)],
              [
                t('channelUsage.pending'),
                count(state.usage.pending_potential_requests),
              ],
              [
                t('channelUsage.unknown'),
                count(state.usage.unknown_provenance_requests),
              ],
            ].map(([label, value]) => (
              <div
                key={label}
                className='flex flex-wrap justify-between gap-x-4 gap-y-1'
              >
                <dt className='text-muted-foreground'>{label}</dt>
                <dd className='min-w-0 text-right font-medium break-all tabular-nums'>
                  {value}
                </dd>
              </div>
            ))}
          </dl>
        </>
      )}
      {countersEnabled && (
        <section
          className='mt-6 space-y-3 border-t pt-4'
          aria-label={t('channelCounters.title')}
        >
          <h3 className='text-sm font-medium'>{t('channelCounters.title')}</h3>
          <p className='text-muted-foreground text-sm'>
            {t('channelCounters.note')}
          </p>
          {counters.status === 'loading' && (
            <LoadingState inline message={t('Loading...')} />
          )}
          {counters.status === 'error' && (
            <p className='text-muted-foreground text-sm' role='status'>
              {t(counters.message)}
            </p>
          )}
          {counters.status === 'ready' && (
            <>
              <dl className='space-y-2 text-sm'>
                {[
                  [
                    t('channelCounters.native'),
                    amount(
                      counters.data.native.net_used_credits,
                      counters.data
                    ),
                  ],
                  [
                    t('channelCounters.inherited'),
                    counters.data.inherited === null
                      ? '-'
                      : amount(
                          counters.data.inherited.net_used_credits,
                          counters.data
                        ),
                  ],
                  [
                    t('channelCounters.combined'),
                    amount(
                      counters.data.combined.net_used_credits,
                      counters.data
                    ),
                  ],
                ].map(([label, value]) => (
                  <div
                    key={label}
                    className='flex flex-wrap justify-between gap-x-4 gap-y-1'
                  >
                    <dt className='text-muted-foreground'>{label}</dt>
                    <dd className='min-w-0 text-right break-all tabular-nums'>
                      {value}
                    </dd>
                  </div>
                ))}
              </dl>
              {!counters.data.complete && (
                <p className='text-muted-foreground text-sm'>
                  {t('channelCounters.incomplete')}
                </p>
              )}
            </>
          )}
        </section>
      )}
    </Dialog>
  )
}
