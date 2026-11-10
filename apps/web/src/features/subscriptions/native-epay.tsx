/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Alert, AlertDescription } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  currentCommerceScope,
  formatCredits,
  readPending,
  requestKey,
  savePending,
  type CommerceScope,
} from '@/features/commerce/native-client'
import { nativeAmountDisplay } from '@/features/wallet/lib/native-checkout'

import type { NativePlan } from './native-api'
import {
  cancelEPayOrder,
  confirmEPayOrder,
  epayPendingSchema,
  freezeEPayOrder,
  listEPayPrices,
  navigateEPayCheckout,
  prepareEPayOrder,
  readEPayOrder,
  type EPayOrder,
  type EPayPending,
  type EPayPrice,
} from './native-epay-api'

export function NativeEPaySubscriptions({
  scope,
  plans,
  blocked,
  onBlockedChange,
  onUpdated,
}: {
  scope: CommerceScope
  plans: NativePlan[]
  blocked: boolean
  onBlockedChange: (blocked: boolean) => void
  onUpdated?: () => void
}) {
  const { t, i18n } = useTranslation()
  const [prices, setPrices] = useState<EPayPrice[]>([])
  const [pending, setPending] = useState<EPayPending | null>(null)
  const [order, setOrder] = useState<EPayOrder | null>(null)
  const [ready, setReady] = useState(false)
  const [storageBlocked, setStorageBlocked] = useState(false)
  const [busy, setBusy] = useState(false)
  const [notice, setNotice] = useState('')
  const mounted = useRef(false)
  const running = useRef(false)
  const current = () => mounted.current && currentCommerceScope(scope)

  function remember(value: EPayPending | null) {
    if (!current()) return false
    try {
      savePending(scope, 'subscription-epay', value)
      setPending(value)
      return true
    } catch {
      setStorageBlocked(true)
      return false
    }
  }
  async function acceptRead(value: EPayOrder) {
    if (!current()) return
    setOrder(value)
    if (value.subscription_id && value.granted_at_unix) onUpdated?.()
  }
  async function run(action: () => Promise<void>) {
    if (running.current || !current() || storageBlocked) return
    running.current = true
    setBusy(true)
    setNotice('')
    try {
      await action()
    } catch {
      if (current()) {
        setNotice(
          t('Result unknown. Retry the saved request before starting another.')
        )
      }
    } finally {
      running.current = false
      if (current()) setBusy(false)
    }
  }
  useEffect(() => {
    mounted.current = true
    try {
      const saved = readPending(scope, 'subscription-epay', epayPendingSchema)
      setPending(saved)
      // Recovery is read-only: never prepare or confirm automatically.
      if (saved?.order_id) {
        void run(async () => acceptRead(await readEPayOrder(scope, saved)))
      }
    } catch {
      setStorageBlocked(true)
    }
    setReady(true)
    return () => {
      mounted.current = false
    }
    // The parent remounts this subtree for each account/session.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])
  useEffect(() => {
    onBlockedChange(!ready || !!pending || storageBlocked)
  }, [ready, pending, storageBlocked, onBlockedChange])
  useEffect(() => {
    let active = true
    setPrices([])
    void Promise.all(plans.map((plan) => listEPayPrices(scope, plan.id)))
      .then((catalog) => {
        if (active && current()) setPrices(catalog.flat())
      })
      .catch(() => {
        if (active && current()) setNotice(t('Request failed'))
      })
    return () => {
      active = false
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [plans, scope.userId, scope.sessionId])

  function preview(price: EPayPrice) {
    if (blocked || pending || !ready) return
    void run(async () => {
      const next: EPayPending = {
        operation_key: requestKey(),
        plan_id: price.plan_id,
        channel_id: price.channel_id,
        confirm_attempted: false,
      }
      if (!remember(next)) return
      const frozen = await prepareEPayOrder(scope, next)
      if (
        !current() ||
        !remember({
          ...next,
          order_id: frozen.order_id,
          frozen: freezeEPayOrder(frozen),
        })
      ) {
        return
      }
      setOrder(frozen)
    })
  }
  function recover() {
    if (!pending) return
    void run(async () => {
      if (pending.order_id) {
        await acceptRead(await readEPayOrder(scope, pending))
      } else {
        const frozen = await prepareEPayOrder(scope, pending)
        if (
          current() &&
          remember({
            ...pending,
            order_id: frozen.order_id,
            frozen: freezeEPayOrder(frozen),
          })
        ) {
          setOrder(frozen)
        }
      }
    })
  }
  function confirm() {
    if (
      !pending ||
      !order ||
      blocked ||
      pending.confirm_attempted ||
      pending.cancel_attempted ||
      order.confirmed_at_unix ||
      order.canceled_at_unix ||
      order.proof_id ||
      Number(order.expires_at_unix) <= Date.now() / 1000
    ) {
      return
    }
    void run(async () => {
      const next = { ...pending, confirm_attempted: true }
      // Persist before dispatch; even an acknowledgement loss forbids new confirmation.
      if (!remember(next)) return
      const checkout = await confirmEPayOrder(scope, order)
      if (current() && remember({ ...next, checkout })) {
        await acceptRead(await readEPayOrder(scope, next))
      }
    })
  }
  function continueCheckout() {
    if (!pending?.order_id || blocked) return
    void run(async () => {
      const fresh = await readEPayOrder(scope, pending)
      await acceptRead(fresh)
      if (
        !current() ||
        !fresh.confirmed_at_unix ||
        fresh.canceled_at_unix ||
        fresh.proof_id ||
        fresh.subscription_id
      ) {
        return
      }
      const checkout = fresh.checkout
      if (!checkout) {
        setNotice(t('Checkout recovery could not be verified.'))
        return
      }
      navigateEPayCheckout(scope, checkout)
    })
  }
  function cancel() {
    if (
      !pending?.order_id ||
      !order ||
      pending.confirm_attempted ||
      order.confirmed_at_unix ||
      order.proof_id ||
      blocked
    ) {
      return
    }
    void run(async () => {
      const next = { ...pending, cancel_attempted: true }
      if (remember(next)) await acceptRead(await cancelEPayOrder(scope, next))
    })
  }
  const granted = !!order?.subscription_id && !!order.granted_at_unix
  const canceledUnissued =
    !!order?.canceled_at_unix &&
    !order.confirmed_at_unix &&
    !pending?.confirm_attempted
  const expired = !!order && Number(order.expires_at_unix) <= Date.now() / 1000
  const disabled = !ready || busy || blocked || storageBlocked
  return (
    <section aria-label='EPay' className='space-y-3 border-t pt-4'>
      <h3 className='font-medium'>EPay</h3>
      {notice && (
        <Alert role='alert'>
          <AlertDescription>{notice}</AlertDescription>
        </Alert>
      )}
      {storageBlocked && (
        <Alert role='alert'>
          <AlertDescription>
            {t('Recovery storage is unavailable. No new request can be sent.')}
          </AlertDescription>
        </Alert>
      )}
      <ul className='divide-y'>
        {prices.map((price) => {
          const plan = plans.find(
            (value) =>
              value.id === price.plan_id &&
              value.revision === price.plan_revision
          )
          if (!plan) return null
          return (
            <li
              key={`${price.plan_id}:${price.channel_id}`}
              className='flex flex-wrap items-center justify-between gap-3 py-3'
            >
              <div>
                <p className='font-medium'>{plan.terms.title}</p>
                <p className='text-muted-foreground text-sm'>
                  {price.channel_name ?? `EPay #${price.channel_id}`} ·{' '}
                  {`${price.currency} ${nativeAmountDisplay(price.amount_minor)}`}
                </p>
              </div>
              <Button
                variant='outline'
                disabled={disabled || !!pending}
                onClick={() => preview(price)}
              >
                {t('Preview purchase')} · EPay
              </Button>
            </li>
          )
        })}
      </ul>
      {pending && (
        <Alert>
          <AlertDescription className='space-y-3'>
            <p className='font-medium'>{t('Confirm purchase')} · EPay</p>
            {order && (
              <>
                <p>
                  {order.plan_json.terms.title} · #{order.plan_revision}
                </p>
                <p className='break-all'>
                  {t('Order ID')}: {order.order_id}
                </p>
                <p>
                  {t('Amount')}: {order.currency}{' '}
                  {nativeAmountDisplay(order.amount_minor)}
                </p>
                <p>
                  {t('Quota')}:{' '}
                  {formatCredits(
                    order.plan_json.terms.limit_units,
                    i18n.language
                  )}{' '}
                  {t('Core credits')} ·{' '}
                  {(order.plan_json.terms.duration_months ?? 0) > 0
                    ? `${order.plan_json.terms.duration_months} ${t('months')}`
                    : `${t('Duration (seconds)')}: ${order.plan_json.terms.duration_seconds}`}
                </p>
                <p>
                  {t('Quota reset period')}:{' '}
                  {t(
                    {
                      grant: 'Never',
                      day: 'Daily',
                      week: 'Weekly',
                      month: 'Monthly',
                      anniversary_month: 'Anniversary month',
                      custom: 'Custom',
                    }[order.plan_json.terms.reset_period]
                  )}
                  {order.plan_json.terms.reset_period === 'custom'
                    ? ` (${order.plan_json.terms.reset_seconds} s)`
                    : ''}
                </p>
                <p className='break-words'>
                  {t('Models')}: {order.plan_json.terms.models.join(', ')} ·{' '}
                  {t('Groups')}: {order.plan_json.terms.groups.join(', ')}
                </p>
                <p>
                  {t('Expires')}:{' '}
                  {new Date(
                    Number(order.expires_at_unix) * 1000
                  ).toLocaleString()}
                </p>
                <Badge variant='secondary'>
                  {t(
                    granted
                      ? 'Subscription purchased successfully'
                      : canceledUnissued
                        ? 'Cancelled'
                        : expired && !order.confirmed_at_unix
                          ? 'Expired'
                          : 'Pending'
                  )}
                </Badge>
              </>
            )}
            <div className='flex flex-wrap gap-2'>
              <Button variant='outline' disabled={disabled} onClick={recover}>
                {t(pending.order_id ? 'Refresh' : 'Retry saved request')}
              </Button>
              {order &&
                !order.confirmed_at_unix &&
                !pending.confirm_attempted &&
                !order.canceled_at_unix &&
                !order.proof_id && (
                  <>
                    <Button
                      disabled={
                        disabled || expired || !!pending.cancel_attempted
                      }
                      onClick={confirm}
                    >
                      {t('Confirm')}
                    </Button>
                    <Button
                      variant='outline'
                      disabled={disabled}
                      onClick={cancel}
                    >
                      {t('Cancel order')}
                    </Button>
                  </>
                )}
              {order?.confirmed_at_unix &&
                !order.canceled_at_unix &&
                !order.proof_id &&
                !granted && (
                  <Button
                    disabled={disabled || !order.checkout}
                    onClick={continueCheckout}
                  >
                    {t('Continue to payment')}
                  </Button>
                )}
              {(granted || canceledUnissued) && (
                <Button
                  variant='outline'
                  disabled={disabled}
                  onClick={() => {
                    if (remember(null)) setOrder(null)
                  }}
                >
                  {t('Close')}
                </Button>
              )}
            </div>
          </AlertDescription>
        </Alert>
      )}
    </section>
  )
}
