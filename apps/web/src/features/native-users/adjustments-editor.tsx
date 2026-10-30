/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useEffect, useRef, useState, type FormEvent } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

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
import { useAuthStore } from '@/stores/auth-store'

import { isCurrentNativeUsersScope, type NativeUsersScope } from './access'
import {
  adjustmentDraftSchema,
  adjustUserBalance,
  readAdjustmentReceipt,
  type AdjustmentDraft,
  type AdjustmentReceipt,
} from './adjustments-api'
import { NativeProfileFailure } from './profile-api'

export function NativeBalanceAdjustment({
  scope,
  userId,
  onUpdated,
  onAccessDenied,
}: {
  scope: NativeUsersScope
  userId: string
  onUpdated: () => void
  onAccessDenied: () => void
}) {
  const { t } = useTranslation()
  const auth = useAuthStore((state) => state.auth)
  const [open, setOpen] = useState(false)
  const [amount, setAmount] = useState('')
  const [reason, setReason] = useState('')
  const [pending, setPending] = useState<AdjustmentDraft>()
  const [result, setResult] = useState<AdjustmentReceipt>()
  const [notice, setNotice] = useState('')
  const [busy, setBusy] = useState(false)
  const [storageBlocked, setStorageBlocked] = useState(false)
  const sending = useRef(false)
  // Retain the exact operation through reloads without persisting credentials.
  const storageKey = `lmm-admin-adjustment:${scope.userId}:${userId}`
  useEffect(() => {
    if (!isCurrentNativeUsersScope(scope)) setOpen(false)
  }, [auth.user?.id, auth.user?.role, auth.session?.sid, scope])
  function show() {
    if (!isCurrentNativeUsersScope(scope)) return
    setNotice('')
    setStorageBlocked(false)
    try {
      const raw = sessionStorage.getItem(storageKey)
      if (raw) {
        const saved = adjustmentDraftSchema.parse(JSON.parse(raw))
        setPending(saved)
        setAmount(saved.delta_units)
        setReason(saved.reason)
      }
      setOpen(true)
    } catch {
      setOpen(true)
      setStorageBlocked(true)
      setNotice(t('Request failed'))
    }
  }
  async function execute(draft: AdjustmentDraft, read: boolean) {
    if (sending.current || !isCurrentNativeUsersScope(scope)) return
    sending.current = true
    setBusy(true)
    setNotice('')
    try {
      const receipt = read
        ? await readAdjustmentReceipt(scope, userId, draft)
        : await adjustUserBalance(scope, userId, draft)
      if (!isCurrentNativeUsersScope(scope)) return
      setResult(receipt)
      setPending(undefined)
      // A saved receipt is permanent. Clearing failure cannot cause a duplicate:
      // opening the retained draft only rechecks/retries the same operation key.
      try {
        sessionStorage.removeItem(storageKey)
      } catch {
        /* retain exact key */
      }
      if (receipt.status === 'posted') toast.success(t('Adjustment posted'))
      else toast.error(t('Adjustment rejected'))
      onUpdated()
    } catch (error) {
      if (!isCurrentNativeUsersScope(scope)) return
      const kind =
        error instanceof NativeProfileFailure ? error.kind : 'unknown'
      if (kind === 'forbidden') {
        setOpen(false)
        onAccessDenied()
      } else {
        setNotice(
          t(
            kind === 'conflict'
              ? 'Operation ID is already used for a different adjustment.'
              : 'Save result is unknown. Check the original operation before continuing.'
          )
        )
      }
    } finally {
      sending.current = false
      setBusy(false)
    }
  }
  function submit(event: FormEvent) {
    event.preventDefault()
    if (
      pending ||
      result ||
      busy ||
      storageBlocked ||
      !isCurrentNativeUsersScope(scope)
    ) {
      return
    }
    try {
      const draft = adjustmentDraftSchema.parse({
        operation_key: crypto.randomUUID(),
        delta_units: amount,
        reason,
      })
      // Never dispatch an operation whose recovery record failed to persist.
      sessionStorage.setItem(storageKey, JSON.stringify(draft))
      setPending(draft)
      void execute(draft, false)
    } catch {
      setNotice(t('Invalid input'))
    }
  }
  function close() {
    if (busy) return
    setOpen(false)
    if (!pending) {
      setResult(undefined)
      setAmount('')
      setReason('')
    }
  }
  return (
    <>
      <Button type='button' variant='outline' size='sm' onClick={show}>
        {t('Adjust funds')}
      </Button>
      <Dialog
        open={open}
        onOpenChange={(value) => {
          if (!value) close()
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              {t('Adjust funds')} · {userId}
            </DialogTitle>
            <DialogDescription>
              {t(
                'Positive values grant Core credits; negative values deduct available credits.'
              )}
            </DialogDescription>
          </DialogHeader>
          {notice && (
            <Alert role='alert'>
              <AlertDescription>{notice}</AlertDescription>
            </Alert>
          )}
          {result ? (
            <Alert role='status'>
              <AlertDescription>
                <p>
                  {t(
                    result.status === 'posted'
                      ? 'Adjustment posted'
                      : 'Adjustment rejected'
                  )}
                </p>
                <p className='font-mono break-all'>
                  {result.operation_key} · {result.delta_units}
                </p>
                {result.journal_id && (
                  <p className='font-mono'>#{result.journal_id}</p>
                )}
                {result.rejection && <p>{result.rejection}</p>}
              </AlertDescription>
            </Alert>
          ) : (
            <form onSubmit={submit} className='space-y-4'>
              <div className='space-y-2'>
                <Label htmlFor={`adjustment-amount-${userId}`}>
                  {t('Signed adjustment (Core credits)')}
                </Label>
                <Input
                  id={`adjustment-amount-${userId}`}
                  value={amount}
                  onChange={(event) => setAmount(event.target.value)}
                  pattern='-?[1-9][0-9]*'
                  required
                  maxLength={20}
                  disabled={busy || Boolean(pending)}
                />
              </div>
              <div className='space-y-2'>
                <Label htmlFor={`adjustment-reason-${userId}`}>
                  {t('Reason')}
                </Label>
                <Input
                  id={`adjustment-reason-${userId}`}
                  value={reason}
                  onChange={(event) => setReason(event.target.value)}
                  required
                  maxLength={250}
                  disabled={busy || Boolean(pending)}
                />
              </div>
              {pending && (
                <p className='font-mono text-sm break-all'>
                  {pending.operation_key}
                </p>
              )}
              <DialogFooter>
                <Button
                  type='button'
                  variant='outline'
                  disabled={busy}
                  onClick={close}
                >
                  {t('Close')}
                </Button>
                {pending ? (
                  <>
                    <Button
                      type='button'
                      variant='outline'
                      disabled={busy}
                      onClick={() => void execute(pending, true)}
                    >
                      {t('Check adjustment receipt')}
                    </Button>
                    <Button
                      type='button'
                      disabled={busy}
                      onClick={() => void execute(pending, false)}
                    >
                      {t('Retry same adjustment')}
                    </Button>
                  </>
                ) : (
                  <Button
                    type='submit'
                    disabled={
                      busy || storageBlocked || !amount || !reason.trim()
                    }
                  >
                    {t('Confirm')}
                  </Button>
                )}
              </DialogFooter>
            </form>
          )}
        </DialogContent>
      </Dialog>
    </>
  )
}
