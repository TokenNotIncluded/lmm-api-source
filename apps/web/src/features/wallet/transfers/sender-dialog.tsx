/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { Copy, ExternalLink, RefreshCw } from 'lucide-react'
import { useEffect, useId, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Alert, AlertDescription } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
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
import {
  commerceScope,
  currentCommerceScope,
  type CommerceScope,
} from '@/features/commerce/native-client'
import { toIntlLocale } from '@/i18n/languages'
import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

import {
  cancelTransfer,
  createTransfer,
  getTransferBalance,
  listTransfers,
  queryTransfer,
  queryTransferByID,
  transferLink,
  type TransferBalance,
  type WalletTransfer,
} from './api'
import {
  canShareTransfer,
  readPendingTransfer,
  reconcileTransfer,
  savePendingTransfer,
  trustedConfirmationURL,
  type PendingTransfer,
  type TransferActionResult,
} from './flow'

interface Props {
  open: boolean
  onOpenChange: (open: boolean) => void
  onUpdated?: () => void
}

export function WalletTransferDialog(props: Props) {
  const auth = useAuthStore((state) => state.auth)
  const scope = commerceScope()
  return scope ? (
    <TransferWorkspace
      key={`${auth.user?.id}:${auth.session?.sid}`}
      {...props}
      scope={scope}
    />
  ) : null
}

function TransferWorkspace({
  scope,
  open,
  onOpenChange,
  onUpdated,
}: Props & { scope: CommerceScope }) {
  const { t, i18n } = useTranslation()
  const locale = toIntlLocale(i18n.language)
  const inputId = useId()
  const [amount, setAmount] = useState('')
  const [balance, setBalance] = useState<TransferBalance | null>(null)
  const [transfers, setTransfers] = useState<WalletTransfer[]>([])
  const [pending, setPending] = useState<PendingTransfer | null>(null)
  const [storageBlocked, setStorageBlocked] = useState(false)
  const [loading, setLoading] = useState(true)
  const [balanceFailed, setBalanceFailed] = useState(false)
  const [historyFailed, setHistoryFailed] = useState(false)
  const [more, setMore] = useState(false)
  const [busy, setBusy] = useState(false)
  const [notice, setNotice] = useState('')
  const [copied, setCopied] = useState<number | null>(null)
  const mounted = useRef(true)
  const running = useRef(false)
  const requests = useRef({ generation: 0 })
  const current = () => mounted.current && currentCommerceScope(scope)
  const loadFailed = balanceFailed || historyFailed
  const credits = (value: number) =>
    `${value.toLocaleString(locale)} ${t('Core credits')}`
  const parsedAmount = /^\d+$/.test(amount) ? Number(amount) : Number.NaN
  const validAmount =
    Number.isSafeInteger(parsedAmount) &&
    parsedAmount > 0 &&
    !!balance &&
    parsedAmount <= balance.spendable_credits

  async function load(append = false) {
    if (!current()) return
    const loadingRequests = requests.current
    const generation = ++loadingRequests.generation
    const owns = () => current() && generation === loadingRequests.generation
    setLoading(true)
    const cursor = append ? (transfers.at(-1)?.id ?? 0) : 0
    const [rows, facts] = await Promise.allSettled([
      listTransfers(scope, cursor),
      append ? Promise.resolve(balance) : getTransferBalance(scope),
    ])
    if (!owns()) return
    if (rows.status === 'fulfilled') {
      setTransfers((previous) =>
        append ? [...previous, ...rows.value] : rows.value
      )
      setMore(rows.value.length === 50)
    }
    setHistoryFailed(rows.status === 'rejected')
    if (!append) {
      setBalance(facts.status === 'fulfilled' ? facts.value : null)
      setBalanceFailed(facts.status === 'rejected')
    }
    setLoading(false)
  }
  useEffect(() => {
    const loadingRequests = requests.current
    mounted.current = true
    try {
      setPending(readPendingTransfer(scope.userId))
    } catch {
      setStorageBlocked(true)
    }
    void load()
    return () => {
      mounted.current = false
      loadingRequests.generation++
    }
    // Changing the account or session remounts the workspace.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  function remember(value: PendingTransfer | null) {
    if (!current()) return false
    try {
      savePendingTransfer(scope.userId, value)
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
    try {
      await action()
    } catch {
      if (current()) {
        setNotice(
          t(
            'The result is unknown. Check this call instead of starting it again.'
          )
        )
      }
    } finally {
      running.current = false
      if (current()) setBusy(false)
    }
  }
  async function accept(
    original: PendingTransfer,
    result: TransferActionResult
  ) {
    if (!current()) return
    const reconciled = reconcileTransfer(original, result)
    if (reconciled.kind === 'pending') {
      remember(reconciled.pending)
      return
    }
    remember(null)
    setAmount('')
    if (reconciled.kind === 'conflict') {
      setNotice(t('This transfer has already been claimed.'))
    }
    onUpdated?.()
    await load()
  }
  function create() {
    if (!validAmount || pending || storageBlocked || loading || loadFailed) {
      return
    }
    void run(async () => {
      const original: PendingTransfer = {
        action: 'create',
        requestKey: crypto.randomUUID(),
        quota: parsedAmount,
        step: { kind: 'unknown' },
      }
      if (!remember(original)) return
      await accept(
        original,
        await createTransfer(scope, original.quota, original.requestKey)
      )
    })
  }
  function check() {
    if (!pending || storageBlocked) return
    const original = pending
    void run(async () => {
      const result =
        original.action === 'create'
          ? await queryTransfer(scope, original.requestKey)
          : await queryTransferByID(scope, original.id)
      await accept(original, result)
    })
  }
  function cancel(row: WalletTransfer) {
    if (pending || storageBlocked || loading || loadFailed) return
    void run(async () => {
      const original: PendingTransfer = {
        action: 'cancel',
        id: row.id,
        requestKey: row.request_key,
        quota: row.quota,
        step: { kind: 'unknown' },
      }
      if (!remember(original)) return
      const result = await cancelTransfer(scope, row.id, row.quota)
      if (!current()) return
      if (result.kind === 'cancelled') {
        remember(null)
        setNotice(t('This transfer was cancelled.'))
        onUpdated?.()
        await load()
      } else {
        await accept(original, result)
      }
    })
  }
  function resume(row: WalletTransfer) {
    if (pending || busy || storageBlocked || !row.request_key) return
    remember({
      action: 'create',
      requestKey: row.request_key,
      quota: row.quota,
      step: row.confirmation_url
        ? { kind: 'confirmation', url: row.confirmation_url, id: row.id }
        : { kind: 'unknown' },
    })
  }
  async function copy(row: WalletTransfer) {
    if (!canShareTransfer(row) || !row.token || !current()) return
    try {
      await navigator.clipboard.writeText(transferLink(row.token))
      if (current()) setCopied(row.id)
    } catch {
      if (current()) setNotice(t('Copy failed'))
    }
  }
  function stateLabel(status: WalletTransfer['status']) {
    switch (status) {
      case 'awaiting_confirmation':
        return t('Awaiting confirmation')
      case 'pending':
        return t('Pending')
      case 'claimed':
        return t('Claimed')
      case 'cancelled':
        return t('Cancelled')
    }
  }
  const configuredOrigin = api.defaults.baseURL
    ? new URL(api.defaults.baseURL, window.location.origin).origin
    : undefined
  const confirmation =
    pending?.step.kind === 'confirmation'
      ? trustedConfirmationURL(
          pending.step.url,
          pending.action,
          pending.step.id,
          window.location.origin,
          configuredOrigin
        )
      : null
  const disabled = busy || loading || !!pending || storageBlocked || loadFailed

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className='max-h-[calc(100dvh-2rem)] overflow-y-auto sm:max-w-2xl'>
        <DialogHeader>
          <DialogTitle>{t('Wallet transfer')}</DialogTitle>
          <DialogDescription>
            {t(
              'Create a private link to transfer part of your wallet balance.'
            )}
          </DialogDescription>
        </DialogHeader>
        <div className='min-w-0 space-y-6'>
          {storageBlocked && (
            <Alert role='alert'>
              <AlertDescription>
                {t(
                  'Recovery storage is unavailable. No new request can be sent.'
                )}
              </AlertDescription>
            </Alert>
          )}
          {notice && (
            <Alert role='status'>
              <AlertDescription>{notice}</AlertDescription>
            </Alert>
          )}
          {balanceFailed && (
            <Alert role='alert'>
              <AlertDescription>
                {t('Failed to load wallet balance.')}
              </AlertDescription>
            </Alert>
          )}
          {historyFailed && (
            <Alert role='alert'>
              <AlertDescription>
                {t('Failed to load transfer history.')}
              </AlertDescription>
            </Alert>
          )}
          <div className='flex flex-wrap items-center justify-between gap-3'>
            <div>
              <p className='text-muted-foreground text-sm'>
                {t('Spendable wallet')}
              </p>
              <p className='mt-1 text-xl font-medium tabular-nums'>
                {balance
                  ? credits(balance.spendable_credits)
                  : t('Unavailable')}
              </p>
            </div>
            <Button
              variant='outline'
              disabled={busy || loading}
              onClick={() => void load()}
            >
              <RefreshCw
                aria-hidden='true'
                className={
                  loading ? 'size-4 motion-safe:animate-spin' : 'size-4'
                }
              />
              {t('Refresh')}
            </Button>
          </div>
          {pending ? (
            <section
              className='bg-muted space-y-3 rounded-xl p-4'
              aria-label={t('Awaiting confirmation')}
            >
              <p className='font-medium tabular-nums'>
                {t(
                  pending.action === 'create' ? 'Transfer' : 'Cancel transfer'
                )}{' '}
                · {credits(pending.quota)}
              </p>
              <p className='text-muted-foreground text-sm leading-relaxed'>
                {t(
                  'Finish confirmation, then check this transfer. The original request is kept until its result is known.'
                )}
              </p>
              <div className='flex flex-wrap gap-2'>
                {confirmation && (
                  <Button
                    disabled={busy || storageBlocked}
                    render={
                      <a
                        href={confirmation}
                        target='_blank'
                        rel='noopener noreferrer'
                      />
                    }
                  >
                    <ExternalLink aria-hidden='true' className='size-4' />
                    {t('Open confirmation page')}
                  </Button>
                )}
                <Button
                  variant='outline'
                  disabled={busy || storageBlocked}
                  onClick={check}
                >
                  {t('Refresh payment status')}
                </Button>
              </div>
            </section>
          ) : (
            <form
              className='space-y-3'
              onSubmit={(event) => {
                event.preventDefault()
                create()
              }}
            >
              <Label htmlFor={inputId}>
                {t('Transfer Amount')} · {t('Core credits')}
              </Label>
              <div className='flex flex-wrap gap-3'>
                <Input
                  id={inputId}
                  className='min-w-0 flex-1 basis-48'
                  value={amount}
                  inputMode='numeric'
                  pattern='[0-9]*'
                  maxLength={16}
                  disabled={disabled || !balance}
                  aria-invalid={amount !== '' && !validAmount}
                  onChange={(event) => setAmount(event.target.value)}
                />
                <Button type='submit' disabled={disabled || !validAmount}>
                  {t('Create transfer link')}
                </Button>
              </div>
              {amount !== '' && !validAmount && (
                <p role='alert' className='text-destructive text-sm'>
                  {t('Invalid transfer amount or request')}
                </p>
              )}
            </form>
          )}
          <section
            className='min-w-0 space-y-3'
            aria-labelledby={`${inputId}-history`}
          >
            <h3 id={`${inputId}-history`} className='font-medium'>
              {t('Transfer history')}
            </h3>
            {loading && (
              <p role='status' className='text-muted-foreground text-sm'>
                {t('Loading...')}
              </p>
            )}
            <ul className='divide-y'>
              {transfers.map((row) => (
                <li key={row.id} className='min-w-0 space-y-3 py-4'>
                  <div className='flex flex-wrap items-center justify-between gap-2'>
                    <p className='font-medium tabular-nums'>
                      {credits(row.quota)}
                    </p>
                    <Badge variant='secondary'>{stateLabel(row.status)}</Badge>
                  </div>
                  <p className='text-muted-foreground text-xs tabular-nums'>
                    #{row.id} ·{' '}
                    {new Date(row.created_at * 1000).toLocaleString(locale)}
                  </p>
                  <div className='flex flex-wrap gap-2'>
                    {canShareTransfer(row) && (
                      <Button
                        size='sm'
                        variant='outline'
                        onClick={() => void copy(row)}
                      >
                        <Copy aria-hidden='true' className='size-4' />
                        {t(copied === row.id ? 'Copied' : 'Copy link')}
                      </Button>
                    )}
                    {row.status === 'pending' && (
                      <Button
                        size='sm'
                        variant='outline'
                        disabled={disabled}
                        onClick={() => cancel(row)}
                      >
                        {t('Cancel transfer')}
                      </Button>
                    )}
                    {row.status === 'awaiting_confirmation' && (
                      <Button
                        size='sm'
                        variant='outline'
                        disabled={disabled || !row.request_key}
                        onClick={() => resume(row)}
                      >
                        {t('Review')}
                      </Button>
                    )}
                  </div>
                </li>
              ))}
            </ul>
            {!loading && !historyFailed && transfers.length === 0 && (
              <p className='text-muted-foreground text-sm'>
                {t('No transfers yet.')}
              </p>
            )}
            {more && (
              <Button
                variant='outline'
                disabled={busy || loading}
                onClick={() => void load(true)}
              >
                {t('Load more')}
              </Button>
            )}
          </section>
        </div>
        <DialogFooter>
          <Button variant='outline' onClick={() => onOpenChange(false)}>
            {t('Close')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
