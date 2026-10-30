/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.

For commercial licensing, please contact support@quantumnous.com
*/
import { useNavigate } from '@tanstack/react-router'
import { AlertTriangle, Loader2 } from 'lucide-react'
import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { Dialog } from '@/components/dialog'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { clearAuthenticationIfCurrent } from '@/lib/auth-session'
import { useAuthStore } from '@/stores/auth-store'

import {
  AccountClosureFailure,
  clearPendingClosure,
  closeAccount,
  closureScope,
  prepareClosure,
  readAccountClosure,
  readClosureReceipt,
  readPendingClosure,
  type AccountClosurePending,
  type AccountClosurePreview,
} from '../../account-lifecycle-api'

interface DeleteAccountDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  username: string
}
const blockerText: Record<AccountClosurePreview['blockers'][number], string> = {
  privileged_account:
    'A privileged account requires administrator review before closure.',
  unsettled_balance:
    'Settle your balance, reserved funds, debt and referral credits before closing the account.',
  financial_history_unavailable:
    'The financial opening balance must be verified before closing the account.',
  owned_team:
    'Transfer or close the teams you own before closing your account.',
  model_request_in_progress:
    'Wait for model requests and reconciliation to finish.',
  active_subscription:
    'Your account has an active or future subscription entitlement.',
  payment_or_refund_pending:
    'A payment or refund still requires completion or reconciliation.',
  order_in_progress: 'Complete or refund your pending shop orders first.',
  transfer_pending: 'Complete or cancel your pending transfers first.',
}

export function DeleteAccountDialog({
  open,
  onOpenChange,
  username,
}: DeleteAccountDialogProps) {
  const { t } = useTranslation()
  const navigate = useNavigate()
  const userId = useAuthStore((state) => state.auth.user?.id)
  const sessionId = useAuthStore((state) => state.auth.session?.sid)
  const [loading, setLoading] = useState(false)
  const [confirmation, setConfirmation] = useState('')
  const [view, setView] = useState<AccountClosurePreview | null>(null)
  const [pending, setPending] = useState<AccountClosurePending | null>(null)
  const [error, setError] = useState('')

  useEffect(() => {
    if (!open) return
    let active = true
    setView(null)
    setConfirmation('')
    setError('')
    setLoading(true)
    const load = async () => {
      try {
        const scope = closureScope()
        const draft = readPendingClosure(scope.userId)
        if (active) setPending(draft)
        const result = await readAccountClosure(scope)
        if (active) setView(result)
      } catch (error) {
        if (active) {
          setError(
            error instanceof AccountClosureFailure && error.kind === 'session'
              ? 'Sign in again to manage account security.'
              : 'Unable to verify account closure requirements.'
          )
        }
      } finally {
        if (active) setLoading(false)
      }
    }
    void load()
    return () => {
      active = false
    }
  }, [open, userId, sessionId])

  const completed = (draft: AccountClosurePending) => {
    // The original operation proves closure of this user's entire authority;
    // do not log out an unrelated account that signed in while checking it.
    const auth = useAuthStore.getState().auth
    if (auth.user?.id === draft.user_id) {
      clearAuthenticationIfCurrent({
        userId: auth.user.id,
        sessionId: auth.session?.sid,
      })
    }
    try {
      clearPendingClosure(draft.operation_key, draft.user_id)
    } catch {
      /* A retained receipt can safely be checked again. */
    }
    toast.success(t('Account closed successfully'))
    onOpenChange(false)
    void navigate({ to: '/sign-in' })
  }
  const handleDelete = async () => {
    if (!view || confirmation !== view.username) return
    try {
      setLoading(true)
      setError('')
      const scope = closureScope()
      const draft = prepareClosure(scope)
      setPending(draft)
      await closeAccount(scope, draft, view, confirmation)
      completed(draft)
    } catch (error) {
      if (error instanceof AccountClosureFailure && error.preview) {
        setView(error.preview)
      }
      setError(
        error instanceof AccountClosureFailure && error.kind === 'session'
          ? 'Sign in again to manage account security.'
          : 'Account closure is not confirmed. Check the receipt before trying again.'
      )
    } finally {
      setLoading(false)
    }
  }
  const checkReceipt = async () => {
    if (!pending) return
    setLoading(true)
    try {
      await readClosureReceipt(pending)
      completed(pending)
    } catch {
      setError(
        'Account closure is not confirmed. Check the receipt before trying again.'
      )
    } finally {
      setLoading(false)
    }
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!loading) onOpenChange(next)
      }}
      title={
        <>
          <AlertTriangle className='h-5 w-5' />
          {t('Close account')}
        </>
      }
      description={t(
        'Your account will be closed and all sessions revoked. Financial and audit records are retained for reconciliation.'
      )}
      contentClassName='sm:max-w-lg'
      titleClassName='text-destructive flex items-center gap-2'
      contentHeight='auto'
      bodyClassName='space-y-4'
      footer={
        <>
          <Button
            type='button'
            variant='outline'
            onClick={() => onOpenChange(false)}
            disabled={loading}
          >
            {t('Cancel')}
          </Button>
          {pending && (
            <Button
              type='button'
              variant='outline'
              onClick={checkReceipt}
              disabled={loading}
            >
              {t('Check closure receipt')}
            </Button>
          )}
          <Button
            type='button'
            variant='destructive'
            onClick={handleDelete}
            disabled={
              loading || !view?.allowed || confirmation !== view.username
            }
          >
            {loading && <Loader2 className='mr-2 h-4 w-4 animate-spin' />}
            {t('Close account')}
          </Button>
        </>
      }
    >
      <div className='my-4 space-y-4'>
        <p className='text-muted-foreground text-sm'>
          {t('Account closure requires a sign-in within the last 10 minutes.')}
        </p>
        {error && (
          <Alert variant='destructive'>
            <AlertDescription>{t(error)}</AlertDescription>
          </Alert>
        )}
        {view && view.blockers.length > 0 && (
          <Alert variant='destructive'>
            <AlertDescription>
              <ul className='list-disc space-y-1 pl-4'>
                {view.blockers.map((reason) => (
                  <li key={reason}>{t(blockerText[reason])}</li>
                ))}
              </ul>
            </AlertDescription>
          </Alert>
        )}
        {pending && (
          <p className='text-muted-foreground text-xs break-all'>
            {t('Operation ID')}: {pending.operation_key}
          </p>
        )}
        <div className='space-y-2'>
          <Label htmlFor='closure-confirmation'>
            {t('Type')} <strong>{view?.username ?? username}</strong>{' '}
            {t('to confirm')}
          </Label>
          <Input
            id='closure-confirmation'
            value={confirmation}
            onChange={(event) => setConfirmation(event.target.value)}
            disabled={loading || !view?.allowed}
            placeholder={view?.username ?? username}
            autoComplete='off'
          />
        </div>
      </div>
    </Dialog>
  )
}
