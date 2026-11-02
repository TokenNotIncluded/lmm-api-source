/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { isAxiosError } from 'axios'
import { useEffect, useId, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { ZodError } from 'zod'

import { Dialog } from '@/components/dialog'
import { LoadingState } from '@/components/loading-state'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { toIntlLocale } from '@/i18n/languages'
import { formatExactQuotaInCurrency } from '@/lib/currency'
import { formatTimestampToDate } from '@/lib/format'
import { DEFAULT_CURRENCY_CONFIG } from '@/stores/system-config-store'

import {
  getNativeBalanceConfiguration,
  saveNativeLegacyBalance,
  type NativeBalanceConfiguration,
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
                  ...(counters.data.lifetime
                    ? [
                        [
                          t('channelCounters.lifetime'),
                          counters.data.lifetime.complete
                            ? `${counters.data.lifetime.total_usd} USD`
                            : t('Unknown'),
                        ],
                      ]
                    : []),
                  ...(counters.data.lifetime?.legacy_opening
                    ? [
                        [
                          t('channelCounters.legacy'),
                          `${counters.data.lifetime.legacy_opening.recorded_usd} USD`,
                        ],
                      ]
                    : []),
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
              {counters.data.lifetime?.legacy_opening && (
                <div className='text-muted-foreground space-y-1 text-xs'>
                  <p>{t('channelCounters.legacyNote')}</p>
                  <p>
                    {t('channelCounters.legacySource', {
                      id: counters.data.lifetime.legacy_opening
                        .source_channel_id,
                      raw: count(
                        counters.data.lifetime.legacy_opening.raw_used_quota
                      ),
                      basis: count(
                        counters.data.lifetime.legacy_opening.credits_per_usd
                      ),
                    })}
                  </p>
                </div>
              )}
              {!(
                counters.data.lifetime?.complete ?? counters.data.complete
              ) && (
                <p className='text-muted-foreground text-sm'>
                  {t('channelCounters.incomplete')}
                </p>
              )}
            </>
          )}
        </section>
      )}
      {countersEnabled && scope.role >= 10 && (
        <NativeLegacyBalanceSettings
          key={`${userId}:${sessionId}:${role}:${channelId}`}
          scope={scope}
          channelId={channelId}
        />
      )}
    </Dialog>
  )
}

const legacyProviders = [
  [1, 'OpenAI'],
  [8, 'Custom'],
  [10, 'AIProxy'],
  [12, 'API2GPT'],
  [13, 'AIGC2D'],
  [20, 'OpenRouter'],
  [25, 'Moonshot'],
  [40, 'SiliconFlow'],
  [43, 'DeepSeek'],
] as const

export function NativeLegacyBalanceSettings({
  scope,
  channelId,
}: {
  scope: NativeRoutingScope
  channelId: number
}) {
  const { t } = useTranslation()
  const formId = useId()
  const [opened, setOpened] = useState(false)
  const [snapshot, setSnapshot] = useState<NativeBalanceConfiguration | null>(
    null
  )
  const [kind, setKind] = useState('')
  const [rate, setRate] = useState('')
  const [busy, setBusy] = useState(false)
  const [needsReload, setNeedsReload] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const pending = useRef(false)
  const active = useRef(true)
  useEffect(() => {
    active.current = true
    return () => {
      active.current = false
    }
  }, [])
  const accept = (saved: NativeBalanceConfiguration) => {
    setSnapshot(saved)
    setKind(saved.legacy ? String(saved.legacy.channel_type) : '')
    setRate(saved.legacy?.cny_per_usd ?? '')
    setNeedsReload(false)
  }
  const report = (cause: unknown, writing: boolean) => {
    const status = isAxiosError(cause) ? cause.response?.status : undefined
    setError(
      t(
        cause instanceof ZodError && writing
          ? 'Exchange rate must be greater than 0'
          : status === 403
            ? 'channelBalance.denied'
            : status === 409
              ? 'channelBalance.conflict'
              : 'channelBalance.saveError'
      )
    )
    if (writing && !(cause instanceof ZodError)) setNeedsReload(true)
  }
  async function load() {
    if (pending.current) return
    pending.current = true
    setOpened(true)
    setBusy(true)
    setError(null)
    try {
      const saved = await getNativeBalanceConfiguration(scope, channelId)
      if (active.current) accept(saved)
    } catch (cause) {
      if (active.current) {
        report(cause, false)
        setNeedsReload(true)
      }
    } finally {
      pending.current = false
      if (active.current) setBusy(false)
    }
  }
  async function save(disable = false) {
    if (pending.current || !snapshot || needsReload) return
    pending.current = true
    setBusy(true)
    setError(null)
    try {
      const saved = await saveNativeLegacyBalance(
        scope,
        channelId,
        snapshot.version,
        disable
          ? null
          : {
              channel_type: Number(kind),
              ...(rate !== '' && ['25', '43'].includes(kind)
                ? { cny_per_usd: rate }
                : {}),
            }
      )
      if (active.current) accept(saved)
    } catch (cause) {
      if (active.current) report(cause, true)
    } finally {
      pending.current = false
      if (active.current) setBusy(false)
    }
  }
  const unit =
    ['1', '8', '20', '43'].includes(kind) || (kind === '25' && rate !== '')
      ? 'USD'
      : kind === '25'
        ? 'CNY'
        : kind === '10'
          ? 'points'
          : t('channelBalance.unitUnknown')
  return (
    <section
      className='mt-6 space-y-3 border-t pt-4'
      aria-label={`${t('channelBalance.configure')} · ${t('Automatic')}`}
    >
      {!opened ? (
        <Button variant='outline' onClick={() => void load()}>
          {t('channelBalance.configure')} · {t('Automatic')}
        </Button>
      ) : (
        <form
          className='space-y-3'
          onSubmit={(event) => {
            event.preventDefault()
            if (kind) void save()
          }}
        >
          <h3 className='text-sm font-medium'>
            {t('channelBalance.configure')} · {t('Automatic')}
          </h3>
          <p className='text-muted-foreground text-sm'>
            {t('channelBalance.note')}
          </p>
          {snapshot?.probe && (
            <p className='text-sm' role='status'>
              {t('Manual')}: <code>{snapshot.probe.path}</code> →{' '}
              {t('Automatic')}
            </p>
          )}
          <Label htmlFor={`${formId}-provider`}>{t('Provider')}</Label>
          <select
            id={`${formId}-provider`}
            value={kind}
            disabled={busy || !snapshot || needsReload}
            className='border-input bg-background h-11 w-full rounded-md border px-3 text-sm'
            onChange={(event) => {
              setKind(event.target.value)
              setRate('')
            }}
          >
            <option value=''>{t('None')}</option>
            {legacyProviders.map(([id, name]) => (
              <option key={id} value={id}>
                {id} · {name === 'Custom' ? t('Custom') : name}
              </option>
            ))}
            {kind && !legacyProviders.some(([id]) => String(id) === kind) && (
              <option value={kind}>
                {kind} · {t('Unknown')}
              </option>
            )}
          </select>
          {['25', '43'].includes(kind) && (
            <div className='space-y-2'>
              <Label htmlFor={`${formId}-rate`}>CNY / USD</Label>
              <Input
                id={`${formId}-rate`}
                value={rate}
                onChange={(event) => setRate(event.target.value)}
                autoComplete='off'
                inputMode='decimal'
                disabled={busy || needsReload}
              />
            </div>
          )}
          {kind && (
            <p className='text-muted-foreground text-sm'>
              {t('channelBalance.unit')}: {unit}
            </p>
          )}
          {error && (
            <Alert variant='destructive' role='alert'>
              <AlertDescription>{error}</AlertDescription>
            </Alert>
          )}
          {busy && <LoadingState inline message={t('Loading...')} />}
          <div className='flex flex-wrap gap-2'>
            <Button
              type='submit'
              disabled={busy || !snapshot || !kind || needsReload}
            >
              {t('Save')} · {t('Automatic')}
            </Button>
            <Button
              type='button'
              variant='outline'
              disabled={busy || !snapshot || needsReload}
              onClick={() => void save(true)}
            >
              {t('Disable')}
            </Button>
            <Button
              type='button'
              variant='outline'
              disabled={busy}
              onClick={() => void load()}
            >
              {t('Refresh')}
            </Button>
          </div>
        </form>
      )}
    </section>
  )
}
