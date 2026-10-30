/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */

import axios from 'axios'
import { useCallback, useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { refreshAuthentication } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

import { useCheckoutOwnerKey } from '../hooks/use-checkout-scope'
import {
  nativeAmountDisplay,
  nativeAmountMinor,
  nativeCheckoutFinished,
  nativeCheckoutURL,
  readNativeCheckout,
  saveNativeCheckout,
  type NativeChannel,
  type NativeCheckout,
  type PendingNativeCheckout,
} from '../lib/native-checkout'
import {
  checkoutNativePayment,
  getNativePaymentChannels,
  getNativePaymentStatus,
  quoteNativePayment,
} from '../native-checkout-api'

const providerNames: Record<NativeChannel['provider'], string> = {
  epay: 'EPay',
  stripe: 'Stripe',
  creem: 'Creem',
  waffo: 'Waffo',
  waffo_pancake: 'Waffo Pancake',
  coinbase: 'Coinbase',
}
const paymentCurrency = (channel: NativeChannel) =>
  channel.provider === 'coinbase' ? 'USDC' : channel.currency

function restore(owner: string) {
  try {
    return {
      pending:
        typeof window === 'undefined'
          ? null
          : readNativeCheckout(owner, window.sessionStorage),
      failed: false,
    }
  } catch {
    return { pending: null, failed: true }
  }
}

export function NativeWalletCheckout() {
  const owner = useCheckoutOwnerKey()
  return <NativeWalletCheckoutForm key={owner} owner={owner} />
}

function NativeWalletCheckoutForm({ owner }: { owner: string }) {
  const { t, i18n } = useTranslation()
  const [restored] = useState(() => restore(owner))
  const [pending, setPending] = useState<PendingNativeCheckout | null>(
    restored.pending
  )
  const [storageFailed, setStorageFailed] = useState(restored.failed)
  const [channels, setChannels] = useState<NativeChannel[]>([])
  const [channelsLoading, setChannelsLoading] = useState(true)
  const [channelsFailed, setChannelsFailed] = useState(false)
  const [channelVersion, setChannelVersion] = useState(0)
  const [selectedID, setSelectedID] = useState('')
  const [amount, setAmount] = useState('10.00')
  const [result, setResult] = useState<NativeCheckout | null>(null)
  const [error, setError] = useState<'unknown' | 'auth' | null>(null)
  const [busy, setBusy] = useState(false)
  const running = useRef(false)
  const mounted = useRef(true)
  const selected =
    pending?.channel ??
    channels.find((item) => item.id === selectedID) ??
    channels[0]
  const finished = nativeCheckoutFinished(result)
  const checkoutURL = nativeCheckoutURL(result?.checkout_url)
  const postCheckoutURL =
    result?.checkout_method === 'POST' && checkoutURL
      ? new URL(checkoutURL)
      : null
  const postCheckoutFields = Array.from(postCheckoutURL?.searchParams ?? [])
  if (postCheckoutURL) {
    postCheckoutURL.search = ''
    postCheckoutURL.hash = ''
  }
  const quote = pending?.quote
  const isCurrent = useCallback(() => {
    const { user, session } = useAuthStore.getState().auth
    return (
      mounted.current &&
      owner === JSON.stringify([user?.id ?? null, session?.sid ?? null])
    )
  }, [owner])
  useEffect(() => {
    mounted.current = true
    return () => {
      mounted.current = false
    }
  }, [])

  useEffect(() => {
    const controller = new AbortController()
    setChannelsLoading(true)
    setChannelsFailed(false)
    void getNativePaymentChannels(controller.signal)
      .then((data) => {
        if (isCurrent()) setChannels(data.available ? data.channels : [])
      })
      .catch(() => {
        if (isCurrent() && !controller.signal.aborted) setChannelsFailed(true)
      })
      .finally(() => {
        if (isCurrent() && !controller.signal.aborted) setChannelsLoading(false)
      })
    return () => controller.abort()
  }, [channelVersion, isCurrent])

  const remember = (value: PendingNativeCheckout | null) => {
    try {
      saveNativeCheckout(owner, value, window.sessionStorage)
      setPending(value)
      return true
    } catch {
      setStorageFailed(true)
      return false
    }
  }
  const fail = useCallback((failure: unknown) => {
    setError(
      axios.isAxiosError(failure) && failure.response?.status === 401
        ? 'auth'
        : 'unknown'
    )
  }, [])
  const run = async (action: () => Promise<void>) => {
    if (running.current || !isCurrent()) return
    running.current = true
    setBusy(true)
    setError(null)
    try {
      await action()
    } catch (failure) {
      if (isCurrent()) fail(failure)
    } finally {
      running.current = false
      if (isCurrent()) setBusy(false)
    }
  }
  const review = () =>
    run(async () => {
      if (!selected) return
      const minor = selected.fixed_amount_minor ?? nativeAmountMinor(amount)
      if (!minor) return
      const next = pending ?? {
        channel: selected,
        request: {
          channel_id: selected.id,
          currency: selected.currency,
          amount_minor: minor,
          request_key: crypto.randomUUID(),
        },
        submitted: false,
      }
      // Preserve identity before the first POST, including a lost quote response.
      if (!remember(next)) return
      const response = await quoteNativePayment(next)
      if (isCurrent()) remember({ ...next, quote: response })
    })
  const confirm = () =>
    run(async () => {
      if (!pending?.quote) return
      const next = { ...pending, quote: pending.quote, submitted: true }
      if (!remember(next)) return
      const response = await checkoutNativePayment(next)
      if (isCurrent()) setResult(response)
    })
  const refresh = () =>
    run(async () => {
      if (!quote) return
      const response = await getNativePaymentStatus(quote)
      if (isCurrent()) setResult(response)
    })

  useEffect(() => {
    if (!pending?.submitted || !quote || finished || busy) return
    let active = true
    let attempts = 0
    let timer: ReturnType<typeof setTimeout>
    const poll = async () => {
      try {
        const response = await getNativePaymentStatus(quote)
        if (!active || !isCurrent()) return
        setResult(response)
        setError(null)
        if (!nativeCheckoutFinished(response) && ++attempts < 40) {
          timer = setTimeout(poll, 3_000)
        }
      } catch (failure) {
        if (active && isCurrent()) fail(failure)
      }
    }
    timer = setTimeout(poll, 3_000)
    return () => {
      active = false
      clearTimeout(timer)
    }
  }, [pending?.submitted, quote, finished, busy, isCurrent, fail])

  const statusMessage = () => {
    if (result?.state === 'refunded') {
      return t('This payment has been refunded.')
    }
    if (result?.state === 'credited' && result.core_credit_confirmed) {
      return t('Funds have reached your wallet.')
    }
    if (result?.state === 'review') {
      return t('This order needs review. Keep its order ID for support.')
    }
    if (
      result?.state === 'refund_pending' ||
      result?.state === 'partially_refunded'
    ) {
      return t('A refund is being processed for this order.')
    }
    if (result?.provider_paid && !result.core_credit_confirmed) {
      return t('Payment received. Waiting for wallet credit.')
    }
    if (result?.state === 'awaiting_payment') {
      return t('Ready to pay. Open the payment page to continue.')
    }
    return t('Preparing your payment. You can refresh this order later.')
  }
  const channelLabel = (channel: NativeChannel) =>
    `${providerNames[channel.provider]}${channel.payment_type ? ` · ${channel.payment_type}` : ''} · ${paymentCurrency(channel)}${channel.environment === 'test' ? ` · ${t('Test payment')}` : ''}`
  const startAgain = () => {
    if (remember(null)) {
      setResult(null)
      setError(null)
    }
  }

  return (
    <section
      aria-labelledby='native-checkout-title'
      className='bg-card space-y-6 rounded-2xl p-5 sm:p-6'
    >
      <div className='space-y-2'>
        <h2 id='native-checkout-title' className='text-lg font-semibold'>
          {t('Add funds')}
        </h2>
        <p className='text-muted-foreground text-sm'>
          {t('Review the exact charge and wallet credit before paying.')}
        </p>
      </div>
      {storageFailed ? (
        <p role='alert' className='text-destructive text-sm'>
          {t(
            'The saved payment could not be read or stored. Keep this page open and contact support before starting another payment.'
          )}
        </p>
      ) : null}
      {!pending && !storageFailed ? (
        channelsLoading ? (
          <p role='status'>{t('Loading payment methods…')}</p>
        ) : channelsFailed || channels.length === 0 ? (
          <div className='space-y-3'>
            <p className='text-muted-foreground text-sm'>
              {t('No payment channel is available right now.')}
            </p>
            <Button
              variant='outline'
              onClick={() => setChannelVersion((value) => value + 1)}
            >
              {t('Refresh')}
            </Button>
          </div>
        ) : (
          <form
            className='space-y-5'
            onSubmit={(event) => {
              event.preventDefault()
              void review()
            }}
          >
            <div className='grid gap-4 sm:grid-cols-2'>
              <div className='space-y-2'>
                <Label htmlFor='native-payment-channel'>
                  {t('Payment method')}
                </Label>
                <NativeSelect
                  id='native-payment-channel'
                  className='w-full'
                  value={selected?.id ?? ''}
                  disabled={busy}
                  onChange={(event) => setSelectedID(event.target.value)}
                >
                  {channels.map((channel) => (
                    <NativeSelectOption key={channel.id} value={channel.id}>
                      {channelLabel(channel)}
                    </NativeSelectOption>
                  ))}
                </NativeSelect>
                {selected?.environment === 'test' ? (
                  <Badge variant='outline'>{t('Test payment')}</Badge>
                ) : null}
              </div>
              <div className='space-y-2'>
                <Label htmlFor='native-payment-amount'>
                  {t('Amount')} · {selected && paymentCurrency(selected)}
                </Label>
                <Input
                  id='native-payment-amount'
                  inputMode='decimal'
                  autoComplete='off'
                  value={
                    selected?.fixed_amount_minor
                      ? nativeAmountDisplay(selected.fixed_amount_minor)
                      : amount
                  }
                  readOnly={Boolean(selected?.fixed_amount_minor)}
                  disabled={busy}
                  onChange={(event) => setAmount(event.target.value)}
                />
                {selected?.fixed_amount_minor ? (
                  <p className='text-muted-foreground text-xs'>
                    {t('This product has a fixed payment amount.')}
                  </p>
                ) : null}
              </div>
            </div>
            {selected?.provider === 'coinbase' ? (
              <p className='text-muted-foreground text-sm'>
                {t(
                  'Pay with USDC through Coinbase. This top-up uses 1 USDC = 1 USD.'
                )}
              </p>
            ) : null}
            <Button
              type='submit'
              disabled={
                busy ||
                !selected ||
                (!selected.fixed_amount_minor && !nativeAmountMinor(amount))
              }
            >
              {t('Review payment')}
            </Button>
          </form>
        )
      ) : null}
      {pending ? (
        <div className='space-y-5'>
          <div className='flex flex-wrap items-center gap-2 text-sm'>
            <span>{channelLabel(pending.channel)}</span>
            {pending.channel.environment === 'test' ? (
              <Badge variant='outline'>{t('Test payment')}</Badge>
            ) : null}
          </div>
          <dl className='grid gap-4 sm:grid-cols-2'>
            <div>
              <dt className='text-muted-foreground text-sm'>{t('You pay')}</dt>
              <dd className='text-xl font-semibold break-all tabular-nums'>
                {paymentCurrency(pending.channel)}{' '}
                {nativeAmountDisplay(pending.request.amount_minor)}
              </dd>
            </div>
            {quote ? (
              <div>
                <dt className='text-muted-foreground text-sm'>
                  {t('Wallet credit')}
                </dt>
                <dd className='text-xl font-semibold break-all tabular-nums'>
                  {BigInt(quote.credit_units).toLocaleString(
                    i18n.resolvedLanguage || i18n.language
                  )}{' '}
                  {t('Credits')}
                </dd>
              </div>
            ) : null}
          </dl>
          {pending.channel.provider === 'coinbase' ? (
            <p className='text-muted-foreground text-sm'>
              {t(
                'Pay with USDC through Coinbase. This top-up uses 1 USDC = 1 USD.'
              )}
            </p>
          ) : null}
          {quote ? (
            <p className='text-muted-foreground text-xs break-all'>
              {t('Order ID')}: {quote.order_id}
            </p>
          ) : null}
          {pending.submitted ? (
            <p role='status' className='text-sm'>
              {statusMessage()}
            </p>
          ) : null}
          <div className='flex flex-wrap gap-2'>
            {!quote ? (
              <Button
                disabled={busy || storageFailed}
                onClick={() => void review()}
              >
                {t('Recover the same quote')}
              </Button>
            ) : !pending.submitted ? (
              <>
                <Button
                  disabled={busy || storageFailed}
                  onClick={() => void confirm()}
                >
                  {t('Confirm payment')}
                </Button>
                <Button variant='ghost' disabled={busy} onClick={startAgain}>
                  {t('Edit payment amount')}
                </Button>
              </>
            ) : (
              <>
                {checkoutURL &&
                !finished &&
                !result?.provider_paid &&
                result?.state === 'awaiting_payment' ? (
                  postCheckoutURL ? (
                    <form
                      method='post'
                      action={postCheckoutURL.href}
                      encType='application/x-www-form-urlencoded'
                      acceptCharset='UTF-8'
                      target='_blank'
                      rel='noopener noreferrer'
                    >
                      {postCheckoutFields.map(([name, value], index) => (
                        <input
                          key={`${name}-${index}`}
                          type='hidden'
                          name={name}
                          value={value}
                        />
                      ))}
                      <Button type='submit'>{t('Open payment page')}</Button>
                    </form>
                  ) : (
                    <Button
                      render={
                        <a
                          href={checkoutURL}
                          target='_blank'
                          rel='noopener noreferrer'
                        />
                      }
                    >
                      {t('Open payment page')}
                    </Button>
                  )
                ) : null}
                <Button
                  variant='outline'
                  disabled={busy}
                  onClick={() => void refresh()}
                >
                  {t('Refresh payment status')}
                </Button>
                {!result ? (
                  <Button
                    variant='ghost'
                    disabled={busy || storageFailed}
                    onClick={() => void confirm()}
                  >
                    {t('Retry the same order')}
                  </Button>
                ) : null}
                {finished ? (
                  <Button disabled={busy || storageFailed} onClick={startAgain}>
                    {t('Start another payment')}
                  </Button>
                ) : null}
              </>
            )}
          </div>
        </div>
      ) : null}
      {error ? (
        <div role='alert' className='space-y-3 text-sm'>
          <p>
            {error === 'auth'
              ? t('Restore your session, then explicitly retry this order.')
              : t(
                  'The result is uncertain. Your original payment request is saved; refresh or retry the same order.'
                )}
          </p>
          {error === 'auth' ? (
            <Button
              variant='outline'
              disabled={busy}
              onClick={() =>
                void run(async () => {
                  const outcome = await refreshAuthentication()
                  if (isCurrent() && outcome.kind !== 'authenticated') {
                    setError('auth')
                  }
                })
              }
            >
              {t('Restore session')}
            </Button>
          ) : null}
        </div>
      ) : null}
    </section>
  )
}
