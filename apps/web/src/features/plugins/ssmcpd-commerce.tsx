import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { z } from 'zod'

import { Button } from '@/components/ui/button'
import { toIntlLocale } from '@/i18n/languages'
import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

const decimal = z.string().regex(/^[1-9]\d*$/)
const offerSchema = z.object({
  period: z.string().regex(/^\d{4}-\d{2}$/),
  unit_credits: decimal,
  bytes_per_unit: decimal,
})
const checkoutSchema = offerSchema.extend({
  request_key: z.string().uuid(),
  quantity: z.literal(1),
})
const orderSchema = z.object({
  order_id: z.string().regex(/^[0-9a-f]{64}$/),
  money_state: z.string(),
  grant_state: z.enum([
    'not_dispatched',
    'redeemable',
    'unknown',
    'rejected',
    'granted',
  ]),
  refund_state: z.string().optional(),
  period: z.string(),
  bytes: decimal,
  credits: decimal,
})
const savedSchema = z.object({
  checkout: checkoutSchema,
  orderId: z
    .string()
    .regex(/^[0-9a-f]{64}$/)
    .optional(),
})
type Saved = z.infer<typeof savedSchema>
type Order = z.infer<typeof orderSchema>
const base = '/api/plugins/ssmcpd'
function readSaved(key: string): Saved | null {
  try {
    const value = savedSchema.safeParse(
      JSON.parse(sessionStorage.getItem(key) ?? 'null')
    )
    return value.success ? value.data : null
  } catch {
    return null
  }
}

export function SSMCPDCommerce({
  userId,
  sessionId,
}: {
  userId: number
  sessionId: string
}) {
  const { t, i18n } = useTranslation()
  const cache = useQueryClient()
  const scope = { userId, sessionId }
  const key = 'ssmcpd-checkout:' + userId + ':' + sessionId
  const [saved, setSaved] = useState<Saved | null>(() => readSaved(key))
  const [reason, setReason] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState(false)
  const active = () => {
    const a = useAuthStore.getState().auth
    return a.user?.id === userId && a.session?.sid === sessionId
  }
  const options = {
    authScope: scope,
    skipAuthRefresh: true,
    skipBusinessError: true,
    skipErrorHandler: true,
    disableDuplicate: true,
  } as const
  function keep(value: Saved | null) {
    if (!active()) return
    if (value) sessionStorage.setItem(key, JSON.stringify(value))
    else sessionStorage.removeItem(key)
    setSaved(value)
  }
  const offer = useQuery({
    queryKey: ['ssmcpd', userId, sessionId, 'offer'],
    retry: false,
    gcTime: 0,
    queryFn: async ({ signal }) =>
      offerSchema.parse(
        (await api.get(base + '/offer', { ...options, signal })).data.data
      ),
  })
  const orderKey = ['ssmcpd', userId, sessionId, 'order', saved?.orderId]
  const order = useQuery({
    queryKey: orderKey,
    enabled: !!saved?.orderId,
    retry: false,
    gcTime: 0,
    queryFn: async ({ signal }) =>
      orderSchema.parse(
        (
          await api.get(base + '/orders/' + saved!.orderId, {
            ...options,
            signal,
          })
        ).data.data
      ),
  })
  // Store the exact price, month and idempotency key before the first POST.
  // Reload and lost responses retain the same purchase identity.
  async function submit(redeem: boolean) {
    if (busy || !active()) return
    setBusy(true)
    setError(false)
    try {
      const attempt: Saved | null =
        saved ??
        readSaved(key) ??
        (offer.data
          ? {
              checkout: {
                ...offer.data,
                request_key: crypto.randomUUID(),
                quantity: 1 as const,
              },
            }
          : null)
      if (!attempt) return
      keep(attempt)
      const target = redeem
        ? base + '/orders/' + attempt.orderId + '/redeem'
        : base + '/orders'
      const response = await api.post(
        target,
        redeem ? {} : attempt.checkout,
        options
      )
      if (!active()) return
      const result = orderSchema.parse(response.data.data)
      keep({ checkout: attempt.checkout, orderId: result.order_id })
      cache.setQueryData(
        ['ssmcpd', userId, sessionId, 'order', result.order_id],
        result
      )
      if (result.grant_state === 'granted')
        void cache.invalidateQueries({
          queryKey: ['ssmcpd', userId, sessionId, 'usage'],
        })
    } catch {
      if (active()) setError(true)
    } finally {
      if (active()) setBusy(false)
    }
  }
  async function requestRefund() {
    if (!saved?.orderId || busy || !active() || !reason.trim()) return
    setBusy(true)
    setError(false)
    try {
      const result = await api.post(
        base + '/orders/' + saved.orderId + '/refund-request',
        { reason: reason.trim() },
        options
      )
      if (active()) {
        cache.setQueryData(orderKey, orderSchema.parse(result.data.data))
        setReason('')
      }
    } catch {
      if (active()) setError(true)
    } finally {
      if (active()) setBusy(false)
    }
  }
  const data: Order | undefined = order.data
  const ended =
    data &&
    (data.grant_state === 'granted' ||
      data.grant_state === 'rejected' ||
      data.money_state === 'refunded' ||
      data.money_state === 'cancelled')
  const canRedeem =
    data &&
    !ended &&
    (data.grant_state === 'unknown' ||
      (!data.refund_state &&
        ['paid', 'fulfilled', 'settled', 'settlement_pending'].includes(
          data.money_state
        )))
  const shown = saved?.checkout ?? offer.data
  const units = (v: string) =>
    BigInt(v).toLocaleString(toIntlLocale(i18n.language))
  const labels: Record<string, string> = {
    awaiting_payment: t('Awaiting payment'),
    payment_pending: t('Payment confirmation pending'),
    paid: t('reserved'),
    fulfilled: t('delivered'),
    settlement_pending: t('Pending settlement'),
    settled: t('Settled'),
    refund_pending: t('Refund pending'),
    refunded: t('Refunded'),
    cancelled: t('Cancelled'),
    not_dispatched: t('Not granted'),
    redeemable: t('Ready to redeem'),
    unknown: t('Awaiting the original receipt'),
    granted: t('Granted'),
    rejected: t('Rejected'),
    requested: t('Refund requested'),
    approved: t('Approved'),
    completed: t('Completed'),
  }
  return (
    <section className='space-y-3'>
      <h2 className='font-medium'>{t('Buy monthly traffic')}</h2>
      {shown && (
        <p>
          {shown.period} · {units(shown.bytes_per_unit)} B ·{' '}
          {units(shown.unit_credits)} {t('credits')}
        </p>
      )}
      {offer.isError && !saved && (
        <p role='alert'>
          {t('Traffic offer unavailable.')}{' '}
          <Button variant='outline' onClick={() => void offer.refetch()}>
            {t('Retry')}
          </Button>
        </p>
      )}
      <div className='flex flex-wrap items-center gap-3'>
        {shown && !saved?.orderId && (
          <Button disabled={busy} onClick={() => void submit(false)}>
            {saved ? t('Retry the same purchase') : t('Buy with balance')}
          </Button>
        )}
        <Link to='/wallet' className='text-sm underline underline-offset-4'>
          {t('Top up wallet')}
        </Link>
      </div>
      {saved?.orderId && (
        <div className='space-y-2 text-sm'>
          <p className='break-all'>
            {t('Order ID')} · {saved.orderId}
          </p>
          {data && (
            <dl className='grid grid-cols-[auto_1fr] gap-x-4'>
              <dt>{t('Payment status')}</dt>
              <dd>{labels[data.money_state] ?? t('Unknown status')}</dd>
              <dt>{t('Traffic grant')}</dt>
              <dd>{labels[data.grant_state]}</dd>
              {data.refund_state && (
                <>
                  <dt>{t('Refund status')}</dt>
                  <dd>{labels[data.refund_state] ?? t('Unknown status')}</dd>
                </>
              )}
            </dl>
          )}
          {data &&
            !data.refund_state &&
            ['paid', 'fulfilled', 'settled'].includes(data.money_state) && (
              <details>
                <summary className='cursor-pointer'>
                  {t('Request refund')}
                </summary>
                <div className='mt-2 flex flex-wrap gap-2'>
                  <input
                    className='rounded-md border px-3'
                    aria-label={t('Refund reason')}
                    placeholder={t('Refund reason')}
                    value={reason}
                    maxLength={2000}
                    onChange={(event) => setReason(event.target.value)}
                  />
                  <Button
                    variant='outline'
                    disabled={busy || !reason.trim()}
                    onClick={() => void requestRefund()}
                  >
                    {t('Request refund')}
                  </Button>
                </div>
              </details>
            )}
          <div className='flex flex-wrap gap-2'>
            {canRedeem && (
              <Button disabled={busy} onClick={() => void submit(true)}>
                {data.grant_state === 'unknown'
                  ? t('Reconcile original order')
                  : t('Redeem traffic')}
              </Button>
            )}
            <Button
              variant='outline'
              disabled={busy}
              onClick={() => void order.refetch()}
            >
              {t('Refresh status')}
            </Button>
            {ended && (
              <Button
                variant='outline'
                onClick={() => {
                  keep(null)
                  setError(false)
                  void offer.refetch()
                }}
              >
                {t('New purchase')}
              </Button>
            )}
          </div>
        </div>
      )}
      {(error || order.isError) && (
        <p role='alert'>
          {t(
            'Unable to confirm this order. Retry with the same purchase reference.'
          )}
        </p>
      )}
    </section>
  )
}
