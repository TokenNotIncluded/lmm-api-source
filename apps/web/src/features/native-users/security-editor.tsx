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
import { NativeProfileFailure } from './profile-api'
import {
  manageUserSecurity,
  readUserSecurity,
  type SecurityAction,
  type SecuritySnapshot,
} from './security-api'

export function NativeUserSecurity({
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
  const [snapshot, setSnapshot] = useState<SecuritySnapshot>()
  const [action, setAction] = useState<SecurityAction>('revoke_credentials')
  const [reason, setReason] = useState('')
  const [notice, setNotice] = useState('')
  const [saving, setSaving] = useState(false)
  const operation = useRef({ generation: 0, busy: false })
  const { userId: actor, sessionId, role } = scope

  useEffect(() => {
    if (!isCurrentNativeUsersScope({ userId: actor, sessionId, role })) {
      setOpen(false)
    }
  }, [
    auth.user?.id,
    auth.session?.sid,
    auth.user?.role,
    actor,
    sessionId,
    role,
  ])
  useEffect(() => {
    const captured = { userId: actor, sessionId, role }
    if (!open || !isCurrentNativeUsersScope(captured)) return
    const controller = new AbortController()
    const activeOperation = operation.current
    const generation = ++activeOperation.generation
    setSnapshot(undefined)
    setReason('')
    setNotice('')
    void readUserSecurity(captured, userId, controller.signal).then(
      (value) => {
        if (
          generation === operation.current.generation &&
          !controller.signal.aborted
        ) {
          setSnapshot(value)
          setAction(value.active ? 'disable' : 'enable')
        }
      },
      (error: unknown) => {
        if (
          generation !== operation.current.generation ||
          controller.signal.aborted
        ) {
          return
        }
        if (
          error instanceof NativeProfileFailure &&
          error.kind === 'forbidden'
        ) {
          setOpen(false)
          onAccessDenied()
        } else setNotice(t('Request failed'))
      }
    )
    return () => {
      controller.abort()
      activeOperation.generation++
    }
  }, [open, actor, sessionId, role, userId, onAccessDenied, t])

  async function save(event: FormEvent) {
    event.preventDefault()
    if (
      !snapshot ||
      !reason.trim() ||
      operation.current.busy ||
      !isCurrentNativeUsersScope(scope)
    ) {
      return
    }
    const generation = operation.current.generation
    operation.current.busy = true
    setSaving(true)
    try {
      await manageUserSecurity(scope, snapshot, action, reason)
      if (
        generation !== operation.current.generation ||
        !isCurrentNativeUsersScope(scope)
      ) {
        return
      }
      setOpen(false)
      toast.success(t('User updated successfully'))
      onUpdated()
    } catch (error) {
      if (
        generation !== operation.current.generation ||
        !isCurrentNativeUsersScope(scope)
      ) {
        return
      }
      const kind =
        error instanceof NativeProfileFailure ? error.kind : 'unknown'
      setSnapshot(undefined)
      if (kind === 'forbidden') {
        setOpen(false)
        onAccessDenied()
      } else {
        setNotice(
          t(
            kind === 'unknown'
              ? 'Save result is unknown. Check the original operation before continuing.'
              : 'Reload the page to try again.'
          )
        )
      }
    } finally {
      operation.current.busy = false
      setSaving(false)
    }
  }
  return (
    <>
      <Button
        type='button'
        size='sm'
        variant='outline'
        onClick={() => setOpen(true)}
      >
        {t('Security')}
      </Button>
      <Dialog
        open={open}
        onOpenChange={(value) => {
          if (!saving) setOpen(value)
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              {t('Account actions')} · {userId}
            </DialogTitle>
            <DialogDescription>
              {t('Existing sessions and API keys will be revoked.')}
            </DialogDescription>
          </DialogHeader>
          {notice && (
            <Alert role='alert'>
              <AlertDescription>{notice}</AlertDescription>
            </Alert>
          )}
          {!snapshot && !notice && <p role='status'>{t('Loading')}</p>}
          <form onSubmit={save} className='space-y-5'>
            <div className='space-y-2'>
              <Label htmlFor={`security-action-${userId}`}>{t('Action')}</Label>
              <select
                id={`security-action-${userId}`}
                className='border-input bg-background min-h-11 w-full rounded-md border px-3 py-2 text-sm'
                value={action}
                onChange={(event) =>
                  setAction(event.target.value as SecurityAction)
                }
                disabled={!snapshot || saving}
              >
                <option value={snapshot?.active ? 'disable' : 'enable'}>
                  {t(snapshot?.active ? 'Disable user' : 'Enable user')}
                </option>
                <option value='reset_mfa'>
                  {t('Reset two-factor authentication')}
                </option>
                <option value='revoke_credentials'>
                  {t('Revoke all sessions and API keys')}
                </option>
              </select>
            </div>
            <div className='space-y-2'>
              <Label htmlFor={`security-reason-${userId}`}>{t('Reason')}</Label>
              <Input
                id={`security-reason-${userId}`}
                value={reason}
                onChange={(event) => setReason(event.target.value)}
                maxLength={250}
                required
                disabled={!snapshot || saving}
              />
            </div>
            <DialogFooter>
              <Button
                type='button'
                variant='outline'
                disabled={saving}
                onClick={() => setOpen(false)}
              >
                {t('Cancel')}
              </Button>
              <Button
                type='submit'
                disabled={
                  !snapshot ||
                  !reason.trim() ||
                  saving ||
                  (action === 'enable' && !snapshot.account_active)
                }
              >
                {t('Confirm')}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>
    </>
  )
}
