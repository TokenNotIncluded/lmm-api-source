/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */

import axios from 'axios'
import { X } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { refreshAuthentication } from '@/lib/api'
import {
  displayAmountToQuota,
  formatExactQuotaInCurrency,
  formatMinimumQuotaInCurrency,
} from '@/lib/currency'
import { useAuthStore } from '@/stores/auth-store'
import { useSystemConfigStore } from '@/stores/system-config-store'

import { useCheckoutOwnerKey } from '../hooks/use-checkout-scope'
import {
  nativeAmountDisplay,
  nativeAmountMinor,
  nativeCreditUSDDisplay,
  nativeRequestedCreditUnits,
  nativeDiscountCode,
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
import { PancakeCustomerPortal } from './pancake-customer-portal'

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
const standardQuickAmounts = [
  '5.00',
  '10.00',
  '25.00',
  '50.00',
  '100.00',
] as const

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

interface NativeWalletCheckoutProps {
  onUpdated?: () => void
}

export function NativeWalletCheckout({ onUpdated }: NativeWalletCheckoutProps) {
  const owner = useCheckoutOwnerKey()
  return (
    <NativeWalletCheckoutForm key={owner} owner={owner} onUpdated={onUpdated} />
  )
}

function NativeWalletCheckoutForm({
  owner,
  onUpdated,
}: NativeWalletCheckoutProps & { owner: string }) {
  const { t, i18n } = useTranslation()
  const currencyConfig = useSystemConfigStore((state) => state.config.currency)
  const [restored] = useState(() => restore(owner))
  const [pending, setPending] = useState<PendingNativeCheckout | null>(
    restored.pending
  )
  const [storageFailed, setStorageFailed] = useState(restored.failed)
  const [channels, setChannels] = useState<NativeChannel[]>([])
  const [channelsLoading, setChannelsLoading] = useState(true)
  const [channelsFailed, setChannelsFailed] = useState(false)
  const [portalAvailable, setPortalAvailable] = useState(false)
  const [channelVersion, setChannelVersion] = useState(0)
  const [selectedID, setSelectedID] = useState('')
  const [amount, setAmount] = useState('10.00')
  const [discountCode, setDiscountCode] = useState(() => {
    if (typeof window === 'undefined') return ''
    const codes = new URLSearchParams(window.location.search).getAll(
      'discount_code'
    )
    return codes.length === 1 ? (nativeDiscountCode(codes[0]) ?? '') : ''
  })
  const [result, setResult] = useState<NativeCheckout | null>(null)
  const [error, setError] = useState<'unknown' | 'auth' | 'invalid' | null>(
    null
  )
  const [busy, setBusy] = useState(false)
  const [checkoutClosed, setCheckoutClosed] = useState(false)
  const running = useRef(false)
  const mounted = useRef(true)
  const lastWalletUpdate = useRef<string | NativeCheckout>('')
  const selected =
    pending?.channel ??
    channels.find((item) => item.id === selectedID) ??
    channels[0]
  const creditCurrency = selected?.currency === 'CNY' ? 'CNY' : 'USD'
  const creditRateUnavailable =
    selected?.credit_quantity &&
    creditCurrency === 'CNY' &&
    !Number.isFinite(displayAmountToQuota('1', 'CNY', currencyConfig))
  const cnyCreditUnits =
    selected?.credit_quantity &&
    creditCurrency === 'CNY' &&
    nativeAmountMinor(amount)
      ? displayAmountToQuota(amount, 'CNY', currencyConfig)
      : Number.NaN
  const requestedCreditUnits = selected?.credit_quantity
    ? creditCurrency === 'USD'
      ? nativeRequestedCreditUnits(amount)
      : Number.isSafeInteger(cnyCreditUnits) && cnyCreditUnits > 0
        ? String(cnyCreditUnits)
        : null
    : null
  const minimumCreditUnits = selected?.credit_quantity
    ? selected.min_credit_units
    : undefined
  const quickAmounts =
    creditCurrency === 'USD' &&
    minimumCreditUnits === nativeRequestedCreditUnits('1.00')
      ? ['1.00', ...standardQuickAmounts]
      : standardQuickAmounts
  const belowMinimum =
    !pending &&
    requestedCreditUnits !== null &&
    minimumCreditUnits !== undefined &&
    BigInt(requestedCreditUnits) < BigInt(minimumCreditUnits)
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
  const creditAmount = quote
    ? formatExactQuotaInCurrency(
        quote.credit_units,
        creditCurrency,
        { locale: i18n.resolvedLanguage || i18n.language },
        currencyConfig
      )
    : '-'
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
    void getNativePaymentChannels(controller.signal)
      .then((data) => {
        if (isCurrent()) {
          setChannels(data.available ? data.channels : [])
          setPortalAvailable(data.pancake_portal_available)
        }
      })
      .catch(() => {
        if (isCurrent() && !controller.signal.aborted) setChannelsFailed(true)
      })
      .finally(() => {
        if (isCurrent() && !controller.signal.aborted) setChannelsLoading(false)
      })
    return () => controller.abort()
  }, [channelVersion, isCurrent])

  useEffect(() => {
    if (!finished || !result || !isCurrent()) return
    // The public status has no refund total or revision. Each fresh partial
    // result can reflect another committed refund on the same order.
    const update =
      result.state === 'partially_refunded'
        ? result
        : `${result.order_id}:${result.state}`
    if (lastWalletUpdate.current === update) return
    lastWalletUpdate.current = update
    onUpdated?.()
  }, [finished, isCurrent, onUpdated, result])

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
        : axios.isAxiosError(failure) && failure.response?.status === 400
          ? 'invalid'
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
      let next = pending
      if (!next) {
        if (!selected) return
        const code = selected.credit_quantity
          ? nativeDiscountCode(discountCode)
          : ''
        if (code === null) return
        const credits = requestedCreditUnits
        const minor = selected.credit_quantity
          ? credits && '0'
          : (selected.fixed_amount_minor ?? nativeAmountMinor(amount))
        if (!minor || belowMinimum) return
        next = {
          channel: selected,
          request: {
            channel_id: selected.id,
            currency: selected.currency,
            amount_minor: minor,
            ...(credits ? { requested_credit_units: credits } : {}),
            ...(code ? { discount_code: code } : {}),
            request_key: crypto.randomUUID(),
          },
          submitted: false,
        }
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
    if (result?.state === 'partially_refunded') {
      return t('Part of this payment has been refunded.')
    }
    if (result?.state === 'refund_pending') {
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
      className='wallet-checkout bg-card space-y-5 rounded-2xl p-5 sm:p-6'
    >
      <div className='space-y-1.5'>
        <h2
          id='native-checkout-title'
          className='text-xl font-semibold tracking-tight'
        >
          {t('Add funds')}
        </h2>
        <p className='text-muted-foreground max-w-prose text-sm leading-6'>
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
            <p
              className='text-muted-foreground text-sm'
              role={channelsFailed ? 'alert' : 'status'}
            >
              {channelsFailed
                ? t('Unable to load payment methods')
                : t('No payment channel is available right now.')}
            </p>
            <Button
              variant='outline'
              onClick={() => {
                setChannelsLoading(true)
                setChannelsFailed(false)
                setPortalAvailable(false)
                setChannelVersion((value) => value + 1)
              }}
            >
              {channelsFailed ? t('Retry') : t('Refresh')}
            </Button>
          </div>
        ) : (
          <form
            className='space-y-4'
            onSubmit={(event) => {
              event.preventDefault()
              void review()
            }}
          >
            <div className='wallet-checkout-fields grid gap-4'>
              <div className='space-y-2'>
                <Label htmlFor='native-payment-channel'>
                  {t('Payment method')}
                </Label>
                <NativeSelect
                  id='native-payment-channel'
                  className='h-12 w-full'
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
                  {t(selected?.credit_quantity ? 'Wallet credit' : 'Amount')} ·{' '}
                  {selected?.credit_quantity
                    ? creditCurrency
                    : selected && paymentCurrency(selected)}
                </Label>
                <Input
                  id='native-payment-amount'
                  className='h-12 text-lg font-medium tabular-nums md:text-lg'
                  inputMode='decimal'
                  autoComplete='off'
                  aria-invalid={belowMinimum || undefined}
                  aria-describedby={
                    minimumCreditUnits && !creditRateUnavailable
                      ? 'native-payment-minimum'
                      : undefined
                  }
                  value={
                    selected?.fixed_amount_minor
                      ? nativeAmountDisplay(selected.fixed_amount_minor)
                      : amount
                  }
                  readOnly={Boolean(selected?.fixed_amount_minor)}
                  disabled={busy}
                  onChange={(event) => setAmount(event.target.value)}
                />
                {creditRateUnavailable ? (
                  <p className='text-muted-foreground text-sm' role='status'>
                    {t(
                      'Current currency rates are unavailable. Refresh after the server currency update.'
                    )}
                  </p>
                ) : null}
                {minimumCreditUnits && !creditRateUnavailable ? (
                  <p
                    id='native-payment-minimum'
                    role={belowMinimum ? 'alert' : undefined}
                    className={
                      belowMinimum
                        ? 'text-destructive text-sm'
                        : 'text-muted-foreground text-sm'
                    }
                  >
                    {creditCurrency === 'CNY'
                      ? t('Minimum {{amount}}', {
                          amount:
                            'CNY ' +
                            formatMinimumQuotaInCurrency(
                              Number(minimumCreditUnits),
                              'CNY',
                              {
                                showSymbol: false,
                                locale: i18n.resolvedLanguage || i18n.language,
                              },
                              currencyConfig
                            ),
                        })
                      : t('Minimum wallet credit is USD {{amount}}.', {
                          amount: nativeCreditUSDDisplay(minimumCreditUnits),
                        })}
                  </p>
                ) : null}
                {!selected?.fixed_amount_minor ? (
                  <div
                    role='group'
                    aria-label={t('Amount')}
                    className='flex flex-wrap gap-2 pt-1'
                  >
                    {quickAmounts.map((value) => (
                      <button
                        key={value}
                        type='button'
                        aria-pressed={amount === value}
                        disabled={busy}
                        onClick={() => setAmount(value)}
                        className='focus-visible:outline-ring bg-background text-muted-foreground hover:text-foreground hover:bg-secondary/65 aria-pressed:bg-foreground aria-pressed:text-background min-h-11 min-w-12 rounded-lg px-3 text-sm font-medium tabular-nums transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 disabled:opacity-50'
                      >
                        {Number(value).toString()}
                      </button>
                    ))}
                  </div>
                ) : null}
                {selected?.fixed_amount_minor ? (
                  <p className='text-muted-foreground text-xs'>
                    {t('This product has a fixed payment amount.')}
                  </p>
                ) : null}
              </div>
            </div>
            {selected?.credit_quantity ? (
              <details
                className='wallet-coupon'
                open={discountCode ? true : undefined}
              >
                <summary className='text-muted-foreground hover:text-foreground w-fit cursor-pointer py-2 text-sm font-medium'>
                  {t('Discount code')}
                </summary>
                <Label className='sr-only' htmlFor='native-payment-discount'>
                  {t('Discount code')}
                </Label>
                <Input
                  id='native-payment-discount'
                  className='h-11'
                  autoComplete='off'
                  spellCheck={false}
                  maxLength={64}
                  value={discountCode}
                  disabled={busy}
                  onChange={(event) => setDiscountCode(event.target.value)}
                />
              </details>
            ) : null}
            {selected?.provider === 'coinbase' ? (
              <p className='text-muted-foreground text-sm'>
                {t(
                  'Pay with USDC through Coinbase. This top-up uses 1 USDC = 1 USD.'
                )}
              </p>
            ) : null}
            <Button
              type='submit'
              className='min-h-11 w-full'
              disabled={
                busy ||
                belowMinimum ||
                !selected ||
                (selected.credit_quantity &&
                  nativeDiscountCode(discountCode) === null) ||
                (!selected.fixed_amount_minor &&
                  !(selected.credit_quantity
                    ? requestedCreditUnits
                    : nativeAmountMinor(amount)))
              }
            >
              {t('Review payment')}
            </Button>
          </form>
        )
      ) : null}
      {pending && quote && checkoutClosed ? (
        <div className='flex flex-wrap items-center justify-between gap-3'>
          <span className='text-muted-foreground text-sm'>
            {t('Order ID')}: {quote.order_id}
          </span>
          <Button variant='outline' onClick={() => setCheckoutClosed(false)}>
            {t('Details')}
          </Button>
        </div>
      ) : pending ? (
        <div className='space-y-6'>
          <div className='flex flex-wrap items-center gap-2 text-sm'>
            <span>{channelLabel(pending.channel)}</span>
            {pending.channel.environment === 'test' ? (
              <Badge variant='outline'>{t('Test payment')}</Badge>
            ) : null}
          </div>
          <dl className='wallet-checkout-fields wallet-checkout-summary bg-muted/50 grid gap-6 rounded-xl p-5 sm:p-6'>
            {quote || pending.request.amount_minor !== '0' ? (
              <div>
                <dt className='text-muted-foreground text-sm'>
                  {t('You pay')}
                </dt>
                <dd className='mt-2 text-2xl font-medium tracking-tight [overflow-wrap:anywhere] tabular-nums'>
                  {paymentCurrency(pending.channel)}{' '}
                  {nativeAmountDisplay(
                    quote?.amount_minor ?? pending.request.amount_minor
                  )}
                </dd>
              </div>
            ) : null}
            {pending.request.discount_code ? (
              <div>
                <dt className='text-muted-foreground text-sm'>
                  {t('Discount code')}
                </dt>
                <dd className='mt-2 font-medium [overflow-wrap:anywhere]'>
                  {pending.request.discount_code}
                </dd>
              </div>
            ) : null}
            {quote ? (
              <div>
                <dt className='text-muted-foreground text-sm'>
                  {t('Wallet credit')}
                </dt>
                <dd className='mt-2 text-2xl font-medium tracking-tight [overflow-wrap:anywhere] tabular-nums'>
                  {creditAmount !== '-' ? <div>{creditAmount}</div> : null}
                  <span
                    className={
                      creditAmount !== '-'
                        ? 'text-muted-foreground text-sm'
                        : undefined
                    }
                  >
                    {BigInt(quote.credit_units).toLocaleString(
                      i18n.resolvedLanguage || i18n.language
                    )}{' '}
                    {t('Credits')}
                  </span>
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
            <p className='text-muted-foreground text-xs [overflow-wrap:anywhere]'>
              {t('Order ID')}: {quote.order_id}
            </p>
          ) : null}
          {pending.submitted ? (
            <p role='status' className='text-sm leading-7'>
              {statusMessage()}
            </p>
          ) : null}
          <div className='flex flex-wrap gap-3'>
            {!quote ? (
              <>
                <Button
                  disabled={busy || storageFailed}
                  onClick={() => void review()}
                >
                  {t('Recover the same quote')}
                </Button>
                {error === 'invalid' ? (
                  <Button variant='ghost' disabled={busy} onClick={startAgain}>
                    {t('Edit payment amount')}
                  </Button>
                ) : null}
              </>
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
                <Button
                  variant='destructive'
                  disabled={busy || storageFailed}
                  onClick={startAgain}
                >
                  <X aria-hidden='true' />
                  {t('Cancel')}
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
                {quote && !finished && !result?.provider_paid ? (
                  <Button
                    variant='destructive'
                    disabled={busy}
                    onClick={() => setCheckoutClosed(true)}
                  >
                    <X aria-hidden='true' />
                    {t('Close')}
                  </Button>
                ) : null}
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
              : error === 'invalid'
                ? t('Payment request failed')
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
      {portalAvailable ? <PancakeCustomerPortal owner={owner} /> : null}
    </section>
  )
}
