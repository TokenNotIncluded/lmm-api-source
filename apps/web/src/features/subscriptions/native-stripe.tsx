/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from '@/components/ui/dialog'
import {
  requestKey,
  type CommerceScope,
} from '@/features/commerce/native-client'
import { nativeAmountDisplay } from '@/features/wallet/lib/native-checkout'

import { useNativeExternalSubscription } from './hooks/use-native-external-subscription'
import type { NativePlan } from './native-api'
import { NativeExternalQuote } from './native-external-quote'
import {
  confirmStripeOrder,
  cancelStripeRenewal,
  freezeStripeOrder,
  listStripePrices,
  navigateStripeCheckout,
  stripePendingSchema,
  prepareStripeOrder,
  readStripeOrder,
  type StripePending,
  type StripePrice,
  type StripeView,
} from './native-stripe-api'

export function NativeStripeSubscriptions({
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
  const [cancelOpen, setCancelOpen] = useState(false)
  const [view, setView] = useState<StripeView | null>(null)

  function accept(value: StripeView) {
    if (!current()) return
    setView(value)
    if (value.order.latest_grant_id || value.order.order.subscription_id) {
      onUpdated?.()
    }
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
  } = useNativeExternalSubscription<StripePending, StripePrice>({
    scope,
    area: 'subscription-stripe',
    pendingSchema: stripePendingSchema,
    plans,
    onBlockedChange,
    listPrices: listStripePrices,
    onRecover: async (saved) => accept(await readStripeOrder(scope, saved)),
  })

  async function prepare(value: StripePending) {
    const order = await prepareStripeOrder(scope, value)
    if (
      remember({
        ...value,
        order_id: order.order.order_id,
        frozen: freezeStripeOrder(order),
      })
    ) {
      accept({ order, checkout: null })
    }
  }
  const order = view?.order.order
  const granted =
    !!view && (!!view.order.latest_grant_id || !!order?.subscription_id)
  const canceled =
    !!view &&
    (['canceled', 'incomplete_expired', 'expired'].includes(view.order.state) ||
      !!order?.canceled_at_unix)
  // oxlint-disable-next-line react/purity -- Quote expiry is informational; the server validates confirmation time.
  const expired = !!order && Number(order.expires_at_unix) <= Date.now() / 1000
  const disabled = !ready || busy || blocked || storageBlocked
  function preview(price: StripePrice) {
    if (disabled || pending) return
    void run(async () => {
      const next: StripePending = {
        provider: 'stripe',
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
      if (pending.order_id) accept(await readStripeOrder(scope, pending))
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
    ) {
      return
    }
    void run(async () => {
      const next = { ...pending, confirm_attempted: true }
      // Confirmation creates a hosted checkout. Persist its one-shot intent before dispatch.
      if (remember(next)) accept(await confirmStripeOrder(scope, next))
    })
  }
  function retryConfirmation() {
    if (!pending?.confirm_attempted || disabled) return
    void run(async () => {
      const fresh = await readStripeOrder(scope, pending)
      accept(fresh)
      if (
        fresh.checkout ||
        fresh.order.latest_grant_id ||
        fresh.order.order.subscription_id ||
        ['canceled', 'incomplete_expired', 'expired'].includes(
          fresh.order.state
        ) ||
        fresh.order.order.canceled_at_unix ||
        fresh.order.order.proof_id ||
        (!fresh.order.order.confirmed_at_unix &&
          Number(fresh.order.order.expires_at_unix) <= Date.now() / 1000)
      ) {
        return
      }
      // Go reuses the frozen order/provider reference and enforces its replay window.
      accept(await confirmStripeOrder(scope, pending))
    })
  }
  function continueCheckout() {
    if (!pending || disabled) return
    void run(async () => {
      const fresh = await readStripeOrder(scope, pending)
      accept(fresh)
      if (current()) navigateStripeCheckout(scope, fresh)
    })
  }
  return (
    <section aria-label='Stripe' className='space-y-3 border-t pt-4'>
      <h3 className='font-medium'>Stripe</h3>
      <p className='text-muted-foreground text-sm'>
        {t('Only verified paid invoices grant subscription access.')}
      </p>
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
                  Stripe #{price.channel_id} · {price.currency}{' '}
                  {nativeAmountDisplay(price.amount_minor)}
                </p>
              </div>
              <Button
                variant='outline'
                disabled={disabled || !!pending}
                onClick={() => preview(price)}
              >
                {t('Preview purchase')} · Stripe
              </Button>
            </li>
          )
        })}
      </ul>
      {pending && (
        <Alert>
          <AlertDescription className='space-y-3'>
            <p className='font-medium'>{t('Confirm purchase')} · Stripe</p>
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
                ['expired', 'incomplete_expired'].includes(
                  view?.order.state ?? ''
                )
                  ? 'Expired'
                  : canceled
                    ? 'Cancelled'
                    : granted
                      ? 'Subscription purchased successfully'
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
              {view && (
                <p>
                  {t('Status')}:{' '}
                  {(
                    {
                      created: t('Pending'),
                      active: t('Active'),
                      canceled: t('Cancelled'),
                      expired: t('Expired'),
                      incomplete: t('Incomplete'),
                      incomplete_expired: t('Incomplete expired'),
                      past_due: t('Past due'),
                      unpaid: t('Unpaid'),
                      trialing: t('Trialing'),
                      paused: t('Paused'),
                      scheduled_cancel: t('Cancellation scheduled'),
                    } as Record<string, string>
                  )[view.order.state] ?? view.order.state}{' '}
                  · {t('Subscription')}:{' '}
                  {view.order.provider_subscription_id || '—'}
                </p>
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
              {view?.order.provider_subscription_id &&
                !canceled &&
                view.order.state !== 'scheduled_cancel' && (
                  <Button
                    variant='outline'
                    disabled={disabled}
                    onClick={() => setCancelOpen(true)}
                  >
                    {t('Cancel renewal')}
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
      <Dialog
        open={cancelOpen}
        onOpenChange={(open) => {
          if (!busy) setCancelOpen(open)
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('Cancel renewal')}</DialogTitle>
            <DialogDescription>
              {t(
                'Request cancellation of future renewals. Current benefits follow verified provider events.'
              )}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              variant='outline'
              disabled={busy}
              onClick={() => setCancelOpen(false)}
            >
              {t('Close')}
            </Button>
            <Button
              disabled={disabled}
              onClick={() => {
                if (!pending) return
                void run(async () => {
                  const next = { ...pending, cancel_attempted: true }
                  if (!remember(next)) return
                  await cancelStripeRenewal(scope, next)
                  if (!current()) return
                  setCancelOpen(false)
                  setNotice(
                    t(
                      'Renewal cancellation requested. Refresh to read the verified status.'
                    )
                  )
                  accept(await readStripeOrder(scope, next))
                })
              }}
            >
              {t(pending?.cancel_attempted ? 'Retry saved request' : 'Confirm')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </section>
  )
}
