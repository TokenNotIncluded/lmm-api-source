/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Alert, AlertDescription } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import {
  commerceScope,
  CommerceFailure,
  currentCommerceScope,
  readPending,
  requestKey,
  savePending,
  type CommerceScope,
} from '@/features/commerce/native-client'
import { toIntlLocale } from '@/i18n/languages'
import { useAuthStore } from '@/stores/auth-store'

import {
  cancelNativeRenewal,
  confirmNativeRefund,
  confirmNativeSubscription,
  listNativePlans,
  listNativePurchases,
  previewNativeRefund,
  purchasePendingSchema,
  quoteNativeSubscription,
  readNativeSubscription,
  type NativePlan,
  type NativePurchase,
  type NativeReceipt,
  type SubscriptionPending,
} from './native-api'
import { NativeCreemSubscriptions } from './native-creem'
import { NativeEPaySubscriptions } from './native-epay'
import { NativePancakeSubscriptions } from './native-pancake'
import { NativeStripeSubscriptions } from './native-stripe'

export function NativeWalletSubscriptions({
  onUpdated,
}: {
  onUpdated?: () => void
}) {
  const auth = useAuthStore((state) => state.auth)
  const scope = commerceScope()
  return scope ? (
    <SubscriptionWallet
      key={`${auth.user?.id}:${auth.session?.sid}`}
      scope={scope}
      onUpdated={onUpdated}
    />
  ) : null
}
function SubscriptionWallet({
  scope,
  onUpdated,
}: {
  scope: CommerceScope
  onUpdated?: () => void
}) {
  const { t, i18n } = useTranslation()
  const locale = toIntlLocale(i18n.language)
  const [plans, setPlans] = useState<NativePlan[]>([])
  const [purchases, setPurchases] = useState<NativePurchase[]>([])
  const [loading, setLoading] = useState(true)
  const [creemBlocked, setCreemBlocked] = useState(true)
  const [stripeBlocked, setStripeBlocked] = useState(true)
  const [epayBlocked, setEPayBlocked] = useState(true)
  const [pancakeBlocked, setPancakeBlocked] = useState(true)
  const [pending, setPending] = useState<SubscriptionPending | null>(null)
  const [receipt, setReceipt] = useState<NativeReceipt | null>(null)
  const [notice, setNotice] = useState('')
  const [storageBlocked, setStorageBlocked] = useState(false)
  const [busy, setBusy] = useState(false)
  const [cancel, setCancel] = useState<NativePurchase | null>(null)
  const [refused, setRefused] = useState(false)
  const running = useRef(false)
  const mounted = useRef(true)
  const loadRequests = useRef({ generation: 0 })
  const current = () => mounted.current && currentCommerceScope(scope)
  async function load() {
    const requests = loadRequests.current
    const generation = ++requests.generation
    const owns = () => current() && generation === requests.generation
    setLoading(true)
    try {
      const [catalog, history] = await Promise.all([
        listNativePlans(scope),
        listNativePurchases(scope),
      ])
      if (owns()) {
        setPlans(catalog)
        setPurchases(history)
      }
    } catch {
      if (owns()) setNotice(t('Request failed'))
    } finally {
      if (owns()) setLoading(false)
    }
  }
  useEffect(() => {
    const requests = loadRequests.current
    mounted.current = true
    try {
      setPending(readPending(scope, 'subscription', purchasePendingSchema))
    } catch {
      setStorageBlocked(true)
    }
    void load()
    return () => {
      mounted.current = false
      requests.generation++
    }
    // Account/session changes remount this workspace.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])
  function remember(value: SubscriptionPending | null) {
    try {
      savePending(scope, 'subscription', value)
      setPending(value)
      return true
    } catch {
      setStorageBlocked(true)
      return false
    }
  }
  async function run(action: () => Promise<void>) {
    if (running.current || !current()) return
    running.current = true
    setBusy(true)
    setNotice('')
    setRefused(false)
    try {
      await action()
    } catch (error) {
      if (current()) {
        const denied = error instanceof CommerceFailure && !error.uncertain
        setRefused(denied)
        setNotice(
          t(
            denied
              ? 'Request rejected. Refresh and review before trying again.'
              : 'Result unknown. Retry the saved request before starting another.'
          )
        )
      }
    } finally {
      running.current = false
      if (current()) setBusy(false)
    }
  }
  function preview(plan: NativePlan, renew_from: number | null = null) {
    if (!plan.terms.allow_balance_pay) return
    void run(async () => {
      const next: SubscriptionPending = {
        kind: 'purchase',
        request: { plan_id: plan.id, renew_from, operation_key: requestKey() },
        confirmed: false,
      }
      if (!remember(next)) return
      const quote = await quoteNativeSubscription(scope, next.request)
      if (current()) remember({ ...next, quote })
    })
  }
  function retry() {
    if (!pending) return
    void run(async () => {
      if (pending.kind === 'purchase' && !pending.quote) {
        const quote = await quoteNativeSubscription(scope, pending.request)
        if (current()) remember({ ...pending, quote })
        return
      }
      if (!pending.quote || !remember({ ...pending, confirmed: true })) return
      const response =
        pending.kind === 'purchase'
          ? await confirmNativeSubscription(scope, pending.quote!)
          : await confirmNativeRefund(scope, pending.quote)
      if (!current()) return
      setReceipt(response)
      remember(null)
      onUpdated?.()
      await load()
    })
  }
  function refreshPayment() {
    if (pending?.kind !== 'purchase' || !pending.quote) return
    const quote = pending.quote
    void run(async () => {
      let result: NativeReceipt | null
      try {
        result = await readNativeSubscription(scope, quote)
      } catch {
        // A failed status read cannot prove that the original debit failed.
        throw new CommerceFailure('RESULT_UNKNOWN')
      }
      if (!current()) return
      if (!result) {
        setNotice(
          t('Result unknown. Retry the saved request before starting another.')
        )
        return
      }
      setReceipt(result)
      remember(null)
      onUpdated?.()
      await load()
    })
  }
  function refund(item: NativePurchase) {
    void run(async () => {
      const quote = await previewNativeRefund(scope, item)
      if (current()) {
        remember({
          kind: 'refund',
          purchase_id: item.id,
          quote,
          confirmed: false,
        })
      }
    })
  }
  const disabled =
    busy ||
    loading ||
    !!pending ||
    storageBlocked ||
    epayBlocked ||
    pancakeBlocked ||
    stripeBlocked ||
    creemBlocked
  const amount =
    pending?.kind === 'purchase'
      ? pending.quote?.plan.terms.price_units
      : pending?.quote.amount_units
  const credits = (value: string) =>
    `${BigInt(value).toLocaleString(locale)} ${t('Core credits')}`
  return (
    <Card>
      <CardHeader>
        <CardTitle>{t('Subscriptions')}</CardTitle>
        <CardDescription>
          {t(
            'Renewals are manual. Cancellation keeps paid benefits until expiry.'
          )}
        </CardDescription>
      </CardHeader>
      <CardContent className='space-y-4'>
        {notice && (
          <Alert role='alert'>
            <AlertDescription>{notice}</AlertDescription>
          </Alert>
        )}
        {storageBlocked && (
          <Alert role='alert'>
            <AlertDescription>
              {t(
                'Recovery storage is unavailable. No new request can be sent.'
              )}
            </AlertDescription>
          </Alert>
        )}
        {receipt && (
          <Alert role='status'>
            <AlertDescription>
              {t(
                receipt.status === 'posted'
                  ? 'Subscription purchased successfully'
                  : receipt.status === 'refunded'
                    ? 'Refund successful'
                    : 'Request rejected'
              )}{' '}
              · {credits(receipt.amount_units)}
              {receipt.journal_id ? ` · #${receipt.journal_id}` : ''}
            </AlertDescription>
          </Alert>
        )}
        <Button
          variant='outline'
          size='sm'
          disabled={busy}
          onClick={() => void load()}
        >
          {t('Refresh')}
        </Button>
        {loading && <p role='status'>{t('Loading...')}</p>}
        {pending && (
          <Alert>
            <AlertDescription className='space-y-3'>
              <p className='font-medium'>
                {t(
                  pending.kind === 'refund'
                    ? 'Confirm refund'
                    : 'Confirm purchase'
                )}
                {amount ? ` · ${credits(amount)}` : ''}
              </p>
              {pending.kind === 'refund' ? (
                <p>
                  {t(
                    'Full refunds are available only before any subscription use.'
                  )}
                </p>
              ) : (
                pending.quote && (
                  <div className='space-y-1'>
                    <p>
                      {pending.quote.plan.terms.title} ·{' '}
                      {new Date(pending.quote.starts_at * 1000).toLocaleString(
                        locale
                      )}{' '}
                      —{' '}
                      {new Date(pending.quote.ends_at * 1000).toLocaleString(
                        locale
                      )}
                    </p>
                    <p>
                      {t('Quota')}:{' '}
                      {credits(pending.quote.plan.terms.limit_units)} ·{' '}
                      {t('Quota reset period')}:{' '}
                      {t(
                        {
                          grant: 'Never',
                          day: 'Daily',
                          week: 'Weekly',
                          month: 'Monthly',
                          anniversary_month: 'Anniversary month',
                          custom: 'Custom',
                        }[pending.quote.plan.terms.reset_period]
                      )}
                      {pending.quote.plan.terms.reset_period === 'custom'
                        ? ` (${pending.quote.plan.terms.reset_seconds.toLocaleString(locale)} s)`
                        : ''}
                    </p>
                    <p className='break-words'>
                      {t('Models')}:{' '}
                      {pending.quote.plan.terms.models.join(', ')} ·{' '}
                      {t('Groups')}:{' '}
                      {pending.quote.plan.terms.groups.join(', ')}
                    </p>
                  </div>
                )
              )}
              <div className='flex flex-wrap gap-2'>
                <Button disabled={busy || storageBlocked} onClick={retry}>
                  {t(
                    pending.confirmed || !pending.quote
                      ? 'Retry saved request'
                      : 'Confirm'
                  )}
                </Button>
                {pending.kind === 'purchase' &&
                  pending.quote &&
                  pending.confirmed && (
                    <Button
                      variant='outline'
                      disabled={busy || storageBlocked}
                      onClick={refreshPayment}
                    >
                      {t('Refresh payment status')}
                    </Button>
                  )}
                {(!pending.confirmed || refused) && (
                  <Button
                    variant='outline'
                    disabled={busy}
                    onClick={() => {
                      remember(null)
                      setNotice('')
                      setRefused(false)
                    }}
                  >
                    {t('Cancel')}
                  </Button>
                )}
              </div>
            </AlertDescription>
          </Alert>
        )}
        <ul className='divide-y'>
          {plans
            .filter((plan) => plan.terms.allow_balance_pay)
            .map((plan) => (
              <li
                key={plan.id}
                className='flex flex-wrap items-center justify-between gap-3 py-3'
              >
                <div className='min-w-0'>
                  <p className='font-medium break-words'>{plan.terms.title}</p>
                  <p className='text-muted-foreground text-sm'>
                    {credits(plan.terms.price_units)} ·{' '}
                    {(plan.terms.duration_months ?? 0) > 0
                      ? `${plan.terms.duration_months?.toLocaleString(locale)} ${t('months')}`
                      : `${t('Duration (seconds)')}: ${plan.terms.duration_seconds.toLocaleString(locale)}`}
                  </p>
                  <p className='text-muted-foreground text-sm tabular-nums'>
                    {t('Quota')}: {credits(plan.terms.limit_units)}
                  </p>
                </div>
                <Button disabled={disabled} onClick={() => preview(plan)}>
                  {t('Preview purchase')}
                </Button>
              </li>
            ))}
        </ul>
        {!loading && !plans.length && (
          <p className='text-muted-foreground text-sm'>
            {t('No subscription plans yet')}
          </p>
        )}
        <NativeEPaySubscriptions
          scope={scope}
          plans={plans}
          blocked={
            busy ||
            !!pending ||
            storageBlocked ||
            pancakeBlocked ||
            stripeBlocked ||
            creemBlocked
          }
          onBlockedChange={setEPayBlocked}
          onUpdated={() => {
            onUpdated?.()
            void load()
          }}
        />
        <NativePancakeSubscriptions
          scope={scope}
          plans={plans}
          blocked={
            busy ||
            !!pending ||
            storageBlocked ||
            epayBlocked ||
            stripeBlocked ||
            creemBlocked
          }
          onBlockedChange={setPancakeBlocked}
          onUpdated={() => {
            onUpdated?.()
            void load()
          }}
        />
        <NativeStripeSubscriptions
          scope={scope}
          plans={plans}
          blocked={
            busy ||
            !!pending ||
            storageBlocked ||
            epayBlocked ||
            pancakeBlocked ||
            creemBlocked
          }
          onBlockedChange={setStripeBlocked}
          onUpdated={() => {
            onUpdated?.()
            void load()
          }}
        />
        <NativeCreemSubscriptions
          scope={scope}
          plans={plans}
          blocked={
            busy ||
            !!pending ||
            storageBlocked ||
            epayBlocked ||
            pancakeBlocked ||
            stripeBlocked
          }
          onBlockedChange={setCreemBlocked}
          onUpdated={() => {
            onUpdated?.()
            void load()
          }}
        />
        <h3 className='font-medium'>{t('Subscription records')}</h3>
        <ul className='divide-y'>
          {purchases.map((item) => {
            const renewalPlan = plans.find((plan) => plan.id === item.plan.id)
            return (
              <li key={item.id} className='space-y-2 py-3'>
                <div className='flex flex-wrap items-center gap-2'>
                  <span className='font-medium'>{item.plan.terms.title}</span>
                  <Badge variant='secondary'>
                    {t(
                      item.refund_journal_id
                        ? 'Refunded'
                        : item.entitlement_active
                          ? 'Active'
                          : item.starts_at > Date.now() / 1000
                            ? 'Scheduled'
                            : 'Expired'
                    )}
                  </Badge>
                </div>
                <p className='text-muted-foreground text-sm'>
                  {new Date(item.starts_at * 1000).toLocaleString(locale)} —{' '}
                  {new Date(item.ends_at * 1000).toLocaleString(locale)}
                </p>
                <dl className='grid gap-3 text-sm sm:grid-cols-3'>
                  <div>
                    <dt className='text-muted-foreground'>{t('Quota')}</dt>
                    <dd className='mt-1 tabular-nums'>
                      {credits(item.plan.terms.limit_units)}
                    </dd>
                  </div>
                  <div>
                    <dt className='text-muted-foreground'>{t('Used')}</dt>
                    <dd className='mt-1 tabular-nums'>
                      {credits(item.used_units)}
                    </dd>
                  </div>
                  <div>
                    <dt className='text-muted-foreground'>{t('Reserved')}</dt>
                    <dd className='mt-1 tabular-nums'>
                      {credits(item.reserved_units)}
                    </dd>
                  </div>
                </dl>
                <div className='flex flex-wrap gap-2'>
                  {renewalPlan?.terms.allow_balance_pay && (
                    <Button
                      size='sm'
                      variant='outline'
                      disabled={
                        disabled ||
                        item.renewal_canceled ||
                        !!item.refund_journal_id
                      }
                      onClick={() => preview(renewalPlan, item.id)}
                    >
                      {t('Renew')}
                    </Button>
                  )}
                  <Button
                    size='sm'
                    variant='outline'
                    disabled={disabled || item.renewal_canceled}
                    onClick={() => setCancel(item)}
                  >
                    {t(
                      item.renewal_canceled
                        ? 'Renewal canceled'
                        : 'Cancel renewal'
                    )}
                  </Button>
                  <Button
                    size='sm'
                    variant='outline'
                    disabled={
                      disabled ||
                      !!item.refund_journal_id ||
                      item.used_units !== '0' ||
                      item.reserved_units !== '0'
                    }
                    onClick={() => refund(item)}
                  >
                    {t('Preview refund')}
                  </Button>
                </div>
              </li>
            )
          })}
        </ul>
        {!loading && !purchases.length && (
          <p className='text-muted-foreground text-sm'>
            {t('No data')}
          </p>
        )}
      </CardContent>
      <Dialog
        open={!!cancel}
        onOpenChange={(open) => {
          if (!open && !busy) setCancel(null)
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('Cancel renewal')}</DialogTitle>
            <DialogDescription>
              {t(
                'Renewals are manual. Cancellation keeps paid benefits until expiry.'
              )}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              variant='outline'
              disabled={busy}
              onClick={() => setCancel(null)}
            >
              {t('Close')}
            </Button>
            <Button
              disabled={busy}
              onClick={() =>
                void run(async () => {
                  if (!cancel) return
                  await cancelNativeRenewal(scope, cancel)
                  if (current()) {
                    setCancel(null)
                    await load()
                  }
                })
              }
            >
              {t('Confirm')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </Card>
  )
}
