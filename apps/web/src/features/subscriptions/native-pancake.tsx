/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import {
  currentCommerceScope,
  readPending,
  requestKey,
  savePending,
  type CommerceScope,
} from '@/features/commerce/native-client'
import { nativeAmountDisplay } from '@/features/wallet/lib/native-checkout'

import type { NativePlan } from './native-api'
import { NativeExternalQuote } from './native-external-quote'
import {
  confirmPancakeOrder,
  freezePancakeOrder,
  listPancakePrices,
  navigatePancakeCheckout,
  pancakePendingSchema,
  preparePancakeOrder,
  readPancakeOrder,
  type PancakePending,
  type PancakePrice,
  type PancakeView,
} from './native-pancake-api'

export function NativePancakeSubscriptions({
  scope,
  plans,
  blocked,
  onBlockedChange,
  onUpdated,
}: {
  scope: CommerceScope
  plans: NativePlan[]
  blocked: boolean
  onBlockedChange: (value: boolean) => void
  onUpdated?: () => void
}) {
  const { t } = useTranslation()
  const [prices, setPrices] = useState<PancakePrice[]>([])
  const [pending, setPending] = useState<PancakePending | null>(null)
  const [view, setView] = useState<PancakeView | null>(null)
  const [ready, setReady] = useState(false)
  const [storageBlocked, setStorageBlocked] = useState(false)
  const [busy, setBusy] = useState(false)
  const [notice, setNotice] = useState('')
  const mounted = useRef(false)
  const running = useRef(false)
  const current = () => mounted.current && currentCommerceScope(scope)
  function remember(value: PancakePending | null) {
    if (!current()) return false
    try {
      savePending(scope, 'subscription-pancake', value)
      setPending(value)
      return true
    } catch {
      setStorageBlocked(true)
      return false
    }
  }
  function accept(value: PancakeView) {
    if (!current()) return
    setView(value)
    if (value.order.latest_grant_id || value.order.order.subscription_id)
      onUpdated?.()
  }
  async function run(action: () => Promise<void>) {
    if (running.current || !current() || storageBlocked) return
    running.current = true
    setBusy(true)
    setNotice('')
    try {
      await action()
    } catch {
      if (current())
        setNotice(
          t('Result unknown. Retry the saved request before starting another.')
        )
    } finally {
      running.current = false
      if (current()) setBusy(false)
    }
  }
  async function prepare(value: PancakePending) {
    const order = await preparePancakeOrder(scope, value)
    if (
      remember({
        ...value,
        order_id: order.order.order_id,
        frozen: freezePancakeOrder(order),
      })
    )
      accept({ order, checkout: null })
  }
  useEffect(() => {
    mounted.current = true
    try {
      const saved = readPending(
        scope,
        'subscription-pancake',
        pancakePendingSchema
      )
      setPending(saved)
      // Mount recovery only reads; retrying a lost prepare requires the saved key and an explicit action.
      if (saved?.order_id)
        void run(async () => accept(await readPancakeOrder(scope, saved)))
    } catch {
      setStorageBlocked(true)
    }
    setReady(true)
    return () => {
      mounted.current = false
    }
    // The parent remounts for each authenticated account/session.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])
  useEffect(() => {
    onBlockedChange(!ready || !!pending || storageBlocked)
  }, [ready, pending, storageBlocked, onBlockedChange])
  useEffect(() => {
    let active = true
    setPrices([])
    void Promise.all(plans.map((p) => listPancakePrices(scope, p.id)))
      .then((values) => {
        if (active && current()) setPrices(values.flat())
      })
      .catch(() => {
        if (active && current()) setNotice(t('Request failed'))
      })
    return () => {
      active = false
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [plans, scope.userId, scope.sessionId])
  const order = view?.order.order
  const granted =
    !!view && (!!view.order.latest_grant_id || !!order?.subscription_id)
  const canceled = view?.order.state === 'canceled' || !!order?.canceled_at_unix
  const expired = !!order && Number(order.expires_at_unix) <= Date.now() / 1000
  const disabled = !ready || busy || blocked || storageBlocked
  function preview(price: PancakePrice) {
    if (disabled || pending) return
    void run(async () => {
      const next: PancakePending = {
        provider: 'waffo_pancake',
        operation_key: requestKey(),
        plan_id: price.plan_id,
        channel_id: price.channel_id,
        confirm_attempted: false,
      }
      if (remember(next)) await prepare(next)
    })
  }
  function recover() {
    if (!pending) return
    void run(async () => {
      if (pending.order_id) accept(await readPancakeOrder(scope, pending))
      else await prepare(pending)
    })
  }
  function confirm() {
    if (
      !pending ||
      !order ||
      disabled ||
      pending.confirm_attempted ||
      order.confirmed_at_unix ||
      order.proof_id ||
      granted ||
      canceled ||
      expired
    )
      return
    void run(async () => {
      const next = { ...pending, confirm_attempted: true }
      // Confirmation creates a hosted checkout. Persist its one-shot intent before dispatch.
      if (remember(next)) accept(await confirmPancakeOrder(scope, next))
    })
  }
  function retryConfirmation() {
    if (!pending?.confirm_attempted || disabled) return
    void run(async () => {
      const fresh = await readPancakeOrder(scope, pending)
      accept(fresh)
      if (
        fresh.checkout ||
        fresh.order.latest_grant_id ||
        fresh.order.order.subscription_id ||
        fresh.order.state === 'canceled' ||
        fresh.order.order.canceled_at_unix ||
        fresh.order.order.proof_id ||
        (!fresh.order.order.confirmed_at_unix &&
          Number(fresh.order.order.expires_at_unix) <= Date.now() / 1000)
      )
        return
      // Go reuses the frozen order/provider reference and enforces its replay window.
      accept(await confirmPancakeOrder(scope, pending))
    })
  }
  function continueCheckout() {
    if (!pending || disabled) return
    void run(async () => {
      const fresh = await readPancakeOrder(scope, pending)
      accept(fresh)
      if (current()) navigatePancakeCheckout(scope, fresh)
    })
  }
  return (
    <section aria-label='Pancake' className='space-y-3 border-t pt-4'>
      <h3 className='font-medium'>Pancake</h3>
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
            (p) => p.id === price.plan_id && p.revision === price.plan_revision
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
                  Pancake #{price.channel_id} · {price.currency}{' '}
                  {nativeAmountDisplay(price.amount_minor)}
                </p>
              </div>
              <Button
                variant='outline'
                disabled={disabled || !!pending}
                onClick={() => preview(price)}
              >
                {t('Preview purchase')} · Pancake
              </Button>
            </li>
          )
        })}
      </ul>
      {pending && (
        <Alert>
          <AlertDescription className='space-y-3'>
            <p className='font-medium'>{t('Confirm purchase')} · Pancake</p>
            {order && (
              <NativeExternalQuote
                plan={order.plan_json}
                orderId={order.order_id}
                currency={order.currency}
                amountMinor={order.amount_minor}
                expiresAt={order.expires_at_unix}
              />
            )}
            <p>
              {t(
                granted
                  ? 'Subscription purchased successfully'
                  : canceled
                    ? 'Cancelled'
                    : expired && !order?.confirmed_at_unix
                      ? 'Expired'
                      : 'Pending'
              )}
            </p>
            <div className='flex flex-wrap gap-2'>
              <Button variant='outline' disabled={disabled} onClick={recover}>
                {t(pending.order_id ? 'Refresh' : 'Retry saved request')}
              </Button>
              {order &&
                !order.confirmed_at_unix &&
                !pending.confirm_attempted &&
                !granted &&
                !canceled &&
                !order.proof_id && (
                  <Button disabled={disabled || expired} onClick={confirm}>
                    {t('Confirm')}
                  </Button>
                )}
              {pending.confirm_attempted &&
                view &&
                !view.checkout &&
                !granted &&
                !canceled &&
                !order?.proof_id && (
                  <Button disabled={disabled} onClick={retryConfirmation}>
                    {t('Retry saved request')}
                  </Button>
                )}
              {order?.confirmed_at_unix &&
                !order.proof_id &&
                !granted &&
                !canceled && (
                  <Button
                    disabled={disabled || !view?.checkout}
                    onClick={continueCheckout}
                  >
                    {t('Continue to payment')}
                  </Button>
                )}
              {(granted ||
                canceled ||
                (expired &&
                  !order?.confirmed_at_unix &&
                  !pending.confirm_attempted)) && (
                <Button
                  variant='outline'
                  disabled={disabled}
                  onClick={() => {
                    if (remember(null)) setView(null)
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
