/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useEffect, useRef, useState, type FormEvent } from 'react'
import { useTranslation } from 'react-i18next'

import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import {
  CommerceFailure,
  currentCommerceScope,
  readPending,
  requestKey,
  savePending,
  type CommerceScope,
} from '@/features/commerce/native-client'
import {
  nativeAmountDisplay,
  nativeAmountMinor,
} from '@/features/wallet/lib/native-checkout'
import { ROLE } from '@/lib/roles'
import { useAuthStore } from '@/stores/auth-store'

import type { NativePlan } from './native-api'
import {
  listAdminCreemChannels,
  listAdminCreemPrices,
  pendingCreemPriceSchema,
  saveAdminCreemPrice,
  pendingCreemPriceSchema as validateCreemPrice,
  type CreemAdminChannel,
  type PendingCreemPrice,
} from './native-creem-admin-api'
import type { CreemPrice } from './native-creem-api'

type Props = {
  scope: CommerceScope
  plan: NativePlan | null
  blocked: boolean
  onSelect: (plan: NativePlan | null) => void
  onBlockedChange: (blocked: boolean) => void
  onEditPlan: (plan: NativePlan) => void
}
export function NativeCreemPriceAdmin(props: Props) {
  const role = useAuthStore((state) => state.auth.user?.role ?? 0)
  return role >= ROLE.SUPER_ADMIN ? (
    <PriceAdmin
      key={`${props.scope.userId}:${props.scope.sessionId}:${role}`}
      {...props}
    />
  ) : null
}
function PriceAdmin({
  scope,
  plan,
  blocked,
  onSelect,
  onBlockedChange,
  onEditPlan,
}: Props) {
  const { t } = useTranslation()
  const [channels, setChannels] = useState<CreemAdminChannel[]>([])
  const [prices, setPrices] = useState<CreemPrice[]>([])
  const [channelId, setChannelId] = useState('')
  const [amount, setAmount] = useState('')
  const [providerPriceId, setProviderPriceId] = useState('')
  const [successUrl, setSuccessUrl] = useState('')
  const [cancelUrl, setCancelUrl] = useState('')
  const [interval, setInterval] = useState<'month' | 'year'>('month')
  const [intervalCount, setIntervalCount] = useState('1')
  const [pending, setPending] = useState<PendingCreemPrice | null>(null)
  const [loading, setLoading] = useState(true)
  const [loadFailed, setLoadFailed] = useState(false)
  const [storageBlocked, setStorageBlocked] = useState(false)
  const [busy, setBusy] = useState(false)
  const [notice, setNotice] = useState('')
  const [refused, setRefused] = useState(false)
  const readVersion = useRef(0)
  const mounted = useRef(false)
  const running = useRef(false)
  const current = () =>
    mounted.current &&
    currentCommerceScope(scope) &&
    (useAuthStore.getState().auth.user?.role ?? 0) >= ROLE.SUPER_ADMIN
  const activePlan = pending?.plan ?? plan
  const visiblePlan = useRef(activePlan)
  visiblePlan.current = activePlan
  const channel = channels.find((value) => String(value.id) === channelId)
  const alreadyPriced =
    !!channel &&
    prices.some(
      (price) =>
        price.channel_id === channel.id &&
        price.plan_revision === activePlan?.revision
    )
  function remember(value: PendingCreemPrice | null) {
    if (!current()) return false
    try {
      savePending(scope, 'subscription-creem-prices', value)
      setPending(value)
      return true
    } catch {
      setStorageBlocked(true)
      return false
    }
  }
  async function load(selected: NativePlan) {
    const version = ++readVersion.current
    const valid = () =>
      current() &&
      version === readVersion.current &&
      visiblePlan.current?.id === selected.id &&
      visiblePlan.current.revision === selected.revision
    setLoading(true)
    setLoadFailed(false)
    try {
      const [catalog, configured] = await Promise.all([
        listAdminCreemChannels(scope),
        listAdminCreemPrices(scope, selected.id),
      ])
      if (
        configured.some((price) => price.plan_revision !== selected.revision)
      ) {
        throw new CommerceFailure('PLAN_REVISION_CHANGED', false)
      }
      if (valid()) {
        setChannels(catalog)
        setPrices(configured)
      }
    } catch {
      if (valid()) {
        setLoadFailed(true)
        setNotice(t('Request failed'))
      }
    } finally {
      if (valid()) setLoading(false)
    }
  }
  useEffect(() => {
    mounted.current = true
    try {
      const saved = readPending(
        scope,
        'subscription-creem-prices',
        pendingCreemPriceSchema
      )
      if (saved) {
        setPending(saved)
        onSelect(saved.plan)
        setChannelId(String(saved.request.price.channel_id))
        setAmount(nativeAmountDisplay(saved.request.price.amount_minor))
        setProviderPriceId(saved.request.price.provider_price_id)
        setSuccessUrl(saved.request.price.success_url)
        setCancelUrl(saved.request.price.cancel_url)
        setInterval(saved.request.price.interval)
        setIntervalCount(String(saved.request.price.interval_count))
      }
    } catch {
      setStorageBlocked(true)
    }
    return () => {
      mounted.current = false
    }
    // Parent remounts the administration workspace on identity/role change.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])
  useEffect(() => {
    onBlockedChange(!!pending || busy || storageBlocked)
  }, [pending, busy, storageBlocked, onBlockedChange])
  useEffect(() => {
    if (!plan) return
    setNotice('')
    setChannels([])
    setPrices([])
    if (!pending) {
      setChannelId('')
      setAmount('')
      setProviderPriceId('')
      setSuccessUrl('')
      setCancelUrl('')
      setInterval(plan.terms.duration_months === 12 ? 'year' : 'month')
      setIntervalCount(
        String(
          plan.terms.duration_months === 12
            ? 1
            : plan.terms.duration_months || 1
        )
      )
    }
    void load(plan)
    // The parent selects an immutable plan snapshot; retry always uses its saved command.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [plan])
  async function submit(event: FormEvent) {
    event.preventDefault()
    if (
      running.current ||
      !current() ||
      blocked ||
      storageBlocked ||
      !activePlan
    ) {
      return
    }
    let command = pending
    if (!command) {
      const minor = nativeAmountMinor(amount)
      if (
        loading ||
        loadFailed ||
        !channel ||
        !minor ||
        alreadyPriced ||
        activePlan.archived
      ) {
        setNotice(t('Invalid input'))
        return
      }
      command = {
        plan: activePlan,
        request: {
          operation_key: requestKey(),
          price: {
            plan_id: activePlan.id,
            plan_revision: activePlan.revision,
            channel_id: channel.id,
            currency: channel.currency,
            amount_minor: minor,
            provider_price_id: providerPriceId,
            interval,
            interval_count: Number(intervalCount),
            success_url: successUrl,
            cancel_url: cancelUrl,
          },
        },
      }
    }
    if (!validateCreemPrice.safeParse(command).success) {
      setNotice(t('Invalid input'))
      return
    }
    if (!remember(command)) return
    running.current = true
    setBusy(true)
    setRefused(false)
    setNotice('')
    try {
      await saveAdminCreemPrice(scope, command.request)
      if (current() && remember(null)) {
        setNotice(t('Saved successfully'))
        await load(command.plan)
      }
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
  const locked = busy || !!pending || blocked || storageBlocked
  return (
    <>
      {storageBlocked && (
        <Alert role='alert'>
          <AlertDescription>
            {t('Recovery storage is unavailable. No new request can be sent.')}
          </AlertDescription>
        </Alert>
      )}
      <Dialog
        open={!!activePlan}
        onOpenChange={(open) => {
          if (!open && !busy && (!pending || refused)) {
            if (refused) remember(null)
            onSelect(null)
          }
        }}
      >
        <DialogContent className='max-h-[90dvh] overflow-y-auto sm:max-w-xl'>
          <DialogHeader>
            <DialogTitle>{t('Price')} · Creem</DialogTitle>
            <DialogDescription>
              {activePlan?.terms.title} · {t('Revision')}:{' '}
              {activePlan?.revision}.{' '}
              {t(
                'Creem prices are fixed per plan revision. Edit the plan to create a new revision before changing a price.'
              )}
            </DialogDescription>
          </DialogHeader>
          {notice && (
            <Alert role='status'>
              <AlertDescription>{notice}</AlertDescription>
            </Alert>
          )}
          {loading ? (
            <p role='status'>{t('Loading...')}</p>
          ) : (
            <ul className='divide-y' aria-label={`Creem ${t('Price')}`}>
              {prices.map((price) => (
                <li
                  key={`${price.plan_revision}:${price.channel_id}`}
                  className='py-2'
                >
                  Creem #{price.channel_id} · {price.currency}{' '}
                  {nativeAmountDisplay(price.amount_minor)} · {t('Revision')}:{' '}
                  {price.plan_revision}
                </li>
              ))}
              {!prices.length && !loadFailed && (
                <li className='text-muted-foreground py-2'>{t('No data')}</li>
              )}
            </ul>
          )}
          <form onSubmit={submit} className='space-y-4'>
            <div className='space-y-2'>
              <Label htmlFor='creem-interval'>{t('Billing interval')}</Label>
              <NativeSelect
                id='creem-interval'
                value={interval}
                disabled={locked || alreadyPriced}
                onChange={(event) =>
                  setInterval(event.target.value as typeof interval)
                }
              >
                <NativeSelectOption value='month'>
                  {t('Month')}
                </NativeSelectOption>
                <NativeSelectOption value='year'>
                  {t('Year')}
                </NativeSelectOption>
              </NativeSelect>
              <Label htmlFor='creem-interval-count'>
                {t('Interval count')}
              </Label>
              <Input
                id='creem-interval-count'
                type='number'
                min={1}
                max={365}
                value={intervalCount}
                disabled={locked || alreadyPriced}
                required
                onChange={(event) => setIntervalCount(event.target.value)}
              />
            </div>
            {[
              {
                id: 'creem-provider-price',
                label: t('Product ID'),
                value: providerPriceId,
                update: setProviderPriceId,
              },
              {
                id: 'creem-success-url',
                label: t('Return after payment'),
                value: successUrl,
                update: setSuccessUrl,
              },
              {
                id: 'creem-cancel-url',
                label: t('Return after cancellation'),
                value: cancelUrl,
                update: setCancelUrl,
              },
            ].map((field) => (
              <div key={field.id} className='space-y-2'>
                <Label htmlFor={field.id}>{field.label}</Label>
                <Input
                  id={field.id}
                  value={field.value}
                  disabled={locked || alreadyPriced}
                  required
                  type={field.id.endsWith('url') ? 'url' : 'text'}
                  onChange={(event) => field.update(event.target.value)}
                />
              </div>
            ))}

            <div className='space-y-2'>
              <Label htmlFor='creem-price-channel'>{t('Channel')}</Label>
              <NativeSelect
                id='creem-price-channel'
                value={channelId}
                disabled={locked || loading || loadFailed}
                onChange={(event) => {
                  setChannelId(event.target.value)
                  setAmount('')
                }}
                required
              >
                <NativeSelectOption value=''>{t('Channel')}</NativeSelectOption>
                {channels.map((value) => (
                  <NativeSelectOption key={value.id} value={String(value.id)}>
                    Creem #{value.id} · {value.currency} ·{' '}
                    {value.payment_type ?? value.environment}
                    {prices.some(
                      (price) =>
                        price.channel_id === value.id &&
                        price.plan_revision === activePlan?.revision
                    )
                      ? ` · ${t('Price')}`
                      : ''}
                  </NativeSelectOption>
                ))}
              </NativeSelect>
            </div>
            <p>
              {t('Currency')}:{' '}
              {pending?.request.price.currency ?? channel?.currency ?? '—'}
            </p>
            <div className='space-y-2'>
              <Label htmlFor='creem-price-amount'>{t('Amount')}</Label>
              <Input
                id='creem-price-amount'
                value={amount}
                inputMode='decimal'
                pattern='[0-9]+([.][0-9]{1,2})?'
                disabled={locked || !channel || alreadyPriced}
                onChange={(event) => setAmount(event.target.value)}
                required
              />
            </div>
            {alreadyPriced && (
              <p className='text-muted-foreground text-sm'>
                {t('Revision')}: {activePlan?.revision} · {t('Price')}:{' '}
                {channel?.currency}{' '}
                {nativeAmountDisplay(
                  prices.find((value) => value.channel_id === channel?.id)
                    ?.amount_minor ?? '0'
                )}
              </p>
            )}
            {alreadyPriced && activePlan && (
              <Button
                type='button'
                variant='outline'
                disabled={locked}
                onClick={() => {
                  onSelect(null)
                  onEditPlan(activePlan)
                }}
              >
                {t('Edit Plan')} · {t('Revision')}
              </Button>
            )}
            <DialogFooter>
              <Button
                type='button'
                variant='outline'
                disabled={busy}
                onClick={() => {
                  if (activePlan) void load(activePlan)
                }}
              >
                {t('Refresh')}
              </Button>
              <Button
                type='button'
                variant='outline'
                disabled={busy || (!!pending && !refused)}
                onClick={() => {
                  if (refused) remember(null)
                  onSelect(null)
                }}
              >
                {t('Close')}
              </Button>
              <Button
                type='submit'
                disabled={
                  busy ||
                  blocked ||
                  storageBlocked ||
                  (!pending &&
                    (loading ||
                      loadFailed ||
                      !channel ||
                      alreadyPriced ||
                      !!activePlan?.archived ||
                      !nativeAmountMinor(amount)))
                }
              >
                {t(pending ? 'Retry saved request' : 'Save')}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>
    </>
  )
}
