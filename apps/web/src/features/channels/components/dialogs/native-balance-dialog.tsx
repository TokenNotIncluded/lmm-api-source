/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { isAxiosError } from 'axios'
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Dialog } from '@/components/dialog'
import { LoadingState } from '@/components/loading-state'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { formatTimestampToDate } from '@/lib/format'

import { formatObservedBalance } from '../../lib/channel-utils'
import {
  getNativeChannelBalance,
  getNativeChannelCounters,
  type NativeRoutingScope,
} from '../../native-api'
import type { BalanceObservation } from '../../types'

type BalanceState =
  | { status: 'loading' }
  | { status: 'ready'; observation: BalanceObservation }
  | { status: 'error'; message: string }
function failureMessage(error: unknown, probe = false) {
  const status = isAxiosError(error) ? error.response?.status : undefined
  return status === 501 && probe
    ? 'channelBalance.disabled'
    : status === 403
      ? 'channelBalance.denied'
      : status === 409
        ? 'channelUsage.identity'
        : 'channelBalance.error'
}
export function NativeBalanceDialog({
  scope,
  channelId,
  channelName,
  probeConfigured,
  onClose,
}: {
  scope: NativeRoutingScope
  channelId: number
  channelName: string
  probeConfigured: boolean
  onClose: () => void
}) {
  const { t } = useTranslation()
  const [state, setState] = useState<BalanceState>({ status: 'loading' })
  const [busy, setBusy] = useState(false)
  const [disabled, setDisabled] = useState(false)
  const [notice, setNotice] = useState<string | null>(null)
  const pending = useRef(false)
  const request = useRef<AbortController | null>(null)
  const { userId, sessionId, role } = scope
  useEffect(() => {
    const controller = new AbortController()
    request.current = controller
    void getNativeChannelCounters(
      { userId, sessionId, role },
      channelId,
      controller.signal
    ).then(
      (counters) => {
        if (!controller.signal.aborted) {
          setState({ status: 'ready', observation: counters.balance })
        }
      },
      (error: unknown) => {
        if (!controller.signal.aborted) {
          setState({ status: 'error', message: failureMessage(error) })
        }
      }
    )
    return () => {
      controller.abort()
      request.current = null
    }
  }, [userId, sessionId, role, channelId])
  async function refresh() {
    const controller = request.current
    if (
      !controller ||
      controller.signal.aborted ||
      pending.current ||
      !probeConfigured ||
      disabled
    ) {
      return
    }
    pending.current = true
    setBusy(true)
    setNotice(null)
    try {
      const observation = await getNativeChannelBalance(
        scope,
        channelId,
        controller.signal
      )
      if (!controller.signal.aborted) setState({ status: 'ready', observation })
    } catch (error: unknown) {
      if (controller.signal.aborted) return
      const message = failureMessage(error, true)
      if (message === 'channelBalance.disabled') setDisabled(true)
      if (state.status === 'ready') setNotice(message)
      else setState({ status: 'error', message })
    } finally {
      pending.current = false
      if (!controller.signal.aborted) setBusy(false)
    }
  }
  const date = (timestamp: number | null) =>
    timestamp !== null && timestamp > 0 ? formatTimestampToDate(timestamp) : '-'
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
      title={t('channelBalance.title')}
      description={channelName}
      contentHeight='auto'
      footer={
        <>
          <Button
            variant='outline'
            disabled={
              state.status === 'loading' || busy || disabled || !probeConfigured
            }
            onClick={() => void refresh()}
          >
            {t('Update Balance')}
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
      {(!probeConfigured || disabled) && (
        <p className='text-muted-foreground mb-3 text-sm' role='status'>
          {t('channelBalance.disabled')}
        </p>
      )}
      {state.status === 'loading' && (
        <LoadingState inline message={t('Loading...')} />
      )}
      {state.status === 'error' && (
        <Alert variant='destructive' role='alert'>
          <AlertDescription>{t(state.message)}</AlertDescription>
        </Alert>
      )}
      {notice && (
        <Alert variant='destructive' role='alert'>
          <AlertDescription>{t(notice)}</AlertDescription>
        </Alert>
      )}
      {state.status === 'ready' && (
        <div className='space-y-3'>
          <p className='text-2xl font-semibold break-all tabular-nums'>
            {formatObservedBalance(state.observation)}
          </p>
          {state.observation.unit_label === null && (
            <p className='text-muted-foreground text-sm'>
              {t('channelBalance.unitUnknown')}
            </p>
          )}
          <dl className='space-y-2 text-sm'>
            <div>
              <dt className='text-muted-foreground'>
                {t('channelBalance.source')}
              </dt>
              <dd>{t(`channelBalance.${state.observation.kind}`)}</dd>
            </div>
            <div>
              <dt className='text-muted-foreground'>{t('Last updated:')}</dt>
              <dd>{date(state.observation.balance_updated_time)}</dd>
            </div>
            {state.observation.source_channel_id !== null && (
              <div>
                <dt className='text-muted-foreground'>
                  {t('channelBalance.sourceChannel')}
                </dt>
                <dd>{state.observation.source_channel_id}</dd>
              </div>
            )}
            <div>
              <dt className='text-muted-foreground'>
                {t('channelBalance.recorded')}
              </dt>
              <dd>{date(state.observation.recorded_at)}</dd>
            </div>
          </dl>
        </div>
      )}
    </Dialog>
  )
}
