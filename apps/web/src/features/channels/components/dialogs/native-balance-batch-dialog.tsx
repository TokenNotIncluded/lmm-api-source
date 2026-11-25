/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Dialog } from '@/components/dialog'
import { Button } from '@/components/ui/button'
import { formatTimestampToDate } from '@/lib/format'

import { formatObservedBalance } from '../../lib/channel-utils'
import {
  nativeBalanceFailureKey,
  refreshNativeChannelBalances,
} from '../../lib/native-balance'
import type { NativeRoutingScope } from '../../native-api'
import type { BalanceObservation } from '../../types'

export function NativeBalanceBatchDialog({
  scope,
  channels,
  canProbe,
  onClose,
}: {
  scope: NativeRoutingScope
  channels: Array<{ id: number; name: string }>
  canProbe: boolean
  onClose: () => void
}) {
  const { t } = useTranslation()
  const [busy, setBusy] = useState(false)
  const [results, setResults] = useState<
    Record<number, { observation?: BalanceObservation; message?: string }>
  >({})
  const request = useRef<AbortController | null>(null)
  const active = useRef(true)
  useEffect(() => {
    active.current = true
    return () => {
      active.current = false
      request.current?.abort()
    }
  }, [])
  useEffect(() => {
    if (!canProbe) request.current?.abort()
  }, [canProbe])

  async function refresh() {
    if (request.current || !canProbe) return
    const controller = new AbortController()
    request.current = controller
    setBusy(true)
    try {
      await refreshNativeChannelBalances(
        scope,
        channels.map((channel) => channel.id),
        controller.signal,
        (result) => {
          setResults((previous) => ({
            ...previous,
            [result.channelId]:
              result.status === 'ready'
                ? { observation: result.observation }
                : {
                    ...previous[result.channelId],
                    message: result.message,
                  },
          }))
        }
      )
    } catch (error: unknown) {
      if (active.current && !controller.signal.aborted) {
        setResults((previous) => ({
          ...previous,
          ...Object.fromEntries(
            channels.map(({ id }) => [
              id,
              {
                ...previous[id],
                message: nativeBalanceFailureKey(error, true),
              },
            ])
          ),
        }))
      }
    } finally {
      request.current = null
      if (active.current) setBusy(false)
    }
  }

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
      title={t('Refresh selected balances')}
      description={t('{{count}} selected', { count: channels.length })}
      contentHeight='auto'
      footer={
        <>
          <Button
            variant='outline'
            disabled={busy || !canProbe}
            onClick={() => void refresh()}
          >
            {busy ? t('Loading...') : t('Update Balance')}
          </Button>
          <Button variant='outline' onClick={onClose}>
            {t('Close')}
          </Button>
        </>
      }
    >
      <p className='text-muted-foreground mb-4 text-sm'>
        {t('channelBalance.note')}
      </p>
      {!canProbe && (
        <p className='text-muted-foreground mb-3 text-sm' role='status'>
          {t('Capability unavailable')}
        </p>
      )}
      <ul
        className='divide-border divide-y'
        aria-live='polite'
        aria-busy={busy}
      >
        {channels.map(({ id, name }) => {
          const result = results[id]
          const observation = result?.observation
          return (
            <li key={id} className='min-w-0 space-y-2 py-4'>
              <p className='font-medium break-all'>{name}</p>
              {observation && (
                <div className='space-y-1 text-sm'>
                  <p className='break-all tabular-nums'>
                    {formatObservedBalance(observation)}
                  </p>
                  <p className='text-muted-foreground'>
                    {t('Last updated:')}{' '}
                    {observation.balance_updated_time
                      ? formatTimestampToDate(observation.balance_updated_time)
                      : '-'}
                  </p>
                </div>
              )}
              {result?.message ? (
                <p className='text-destructive text-sm' role='status'>
                  {t(result.message)}
                </p>
              ) : !observation ? (
                <p className='text-muted-foreground text-sm'>
                  {t('Not queried')}
                </p>
              ) : null}
            </li>
          )
        })}
      </ul>
    </Dialog>
  )
}
