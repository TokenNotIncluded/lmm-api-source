/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Alert, AlertDescription } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  requestKey,
  type CommerceScope,
} from '@/features/commerce/native-client'
import { nativeAmountDisplay } from '@/features/wallet/lib/native-checkout'

import { useNativeExternalSubscription } from './hooks/use-native-external-subscription'
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
import { NativeExternalQuote } from './native-external-quote'

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
  const { t } = useTranslation()
  const [order, setOrder] = useState<EPayOrder | null>(null)

  function acceptRead(value: EPayOrder) {
    if (!current()) return
    setOrder(value)
    if (value.subscription_id && value.granted_at_unix) onUpdated?.()
  }

  const {
    prices,
    pending,
    ready,
    storageBlocked,
    busy,
    notice,
    setNotice,
    current,
    remember,
    run,
  } = useNativeExternalSubscription<EPayPending, EPayPrice>({
    scope,
    area: 'subscription-epay',
    pendingSchema: epayPendingSchema,
    plans,
    onBlockedChange,
    listPrices: listEPayPrices,
    onRecover: async (saved) => acceptRead(await readEPayOrder(scope, saved)),
  })

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
                <NativeExternalQuote
                  plan={order.plan_json}
                  orderId={order.order_id}
                  currency={order.currency}
                  amountMinor={order.amount_minor}
                  expiresAt={order.expires_at_unix}
                />
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
