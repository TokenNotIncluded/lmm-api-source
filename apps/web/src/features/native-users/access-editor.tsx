/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useEffect, useRef, useState, type FormEvent } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
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
import { Textarea } from '@/components/ui/textarea'
import { useAuthStore } from '@/stores/auth-store'

import { isCurrentNativeUsersScope, type NativeUsersScope } from './access'
import { readUserAccess, updateUserAccess } from './access-api'
import {
  parseAccessGroups,
  sameAccess,
  userAccessValuesSchema,
  validAccessReason,
  type UserAccessSnapshot,
  type UserAccessValues,
} from './access-contract'
import { NativeProfileFailure } from './profile-api'

export function NativeUserAccess({
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
  const [snapshot, setSnapshot] = useState<UserAccessSnapshot>()
  const [level, setLevel] = useState(0)
  const [developer, setDeveloper] = useState(false)
  const [groups, setGroups] = useState('')
  const [reason, setReason] = useState('')
  const [review, setReview] = useState<UserAccessValues>()
  const [notice, setNotice] = useState<
    '' | 'unavailable' | 'conflict' | 'unknown'
  >('')
  const [saving, setSaving] = useState(false)
  const [reload, setReload] = useState(0)
  const operation = useRef({ generation: 0, busy: false })
  const { userId: actor, sessionId, role } = scope
  const active =
    auth.user?.id === actor &&
    auth.session?.sid === sessionId &&
    auth.user?.role === role

  useEffect(() => {
    const captured = { userId: actor, sessionId, role }
    if (!open || !active || !isCurrentNativeUsersScope(captured)) return
    const controller = new AbortController()
    const holder = operation.current
    const generation = ++holder.generation
    void readUserAccess(captured, userId, controller.signal).then(
      (value) => {
        if (generation !== holder.generation || controller.signal.aborted) {
          return
        }
        setSnapshot(value)
        setLevel(value.service_level)
        setDeveloper(value.developer_access_granted)
        setGroups(value.groups.join('\n'))
      },
      (error: unknown) => {
        if (generation !== holder.generation || controller.signal.aborted) {
          return
        }
        if (
          error instanceof NativeProfileFailure &&
          error.kind === 'forbidden'
        ) {
          setOpen(false)
          onAccessDenied()
        } else {
          setNotice('unavailable')
        }
      }
    )
    return () => {
      controller.abort()
      holder.generation++
    }
  }, [open, active, actor, sessionId, role, userId, reload, onAccessDenied])

  function resetDraft() {
    operation.current.generation++
    setSnapshot(undefined)
    setReview(undefined)
    setReason('')
    setNotice('')
  }

  function changeOpen(next: boolean) {
    if (operation.current.busy) return
    resetDraft()
    setOpen(next)
  }

  const values = {
    service_level: level,
    developer_access_granted: developer,
    groups: parseAccessGroups(groups),
  }
  const valid =
    userAccessValuesSchema.safeParse(values).success &&
    validAccessReason(reason)
  const changed = snapshot && !sameAccess(snapshot, values)

  async function submit(event: FormEvent) {
    event.preventDefault()
    if (
      !snapshot ||
      !snapshot.account_active ||
      !valid ||
      !changed ||
      operation.current.busy ||
      !isCurrentNativeUsersScope(scope)
    ) {
      return
    }
    if (!review) {
      setReview(values)
      return
    }
    const generation = operation.current.generation
    operation.current.busy = true
    setSaving(true)
    setNotice('')
    try {
      await updateUserAccess(scope, snapshot, review, reason)
      if (
        generation !== operation.current.generation ||
        !isCurrentNativeUsersScope(scope)
      ) {
        return
      }
      setOpen(false)
      toast.success(t('User access updated'))
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
      setReview(undefined)
      if (kind === 'forbidden') {
        setOpen(false)
        onAccessDenied()
      } else {
        setNotice(
          kind === 'conflict'
            ? 'conflict'
            : kind === 'unknown'
              ? 'unknown'
              : 'unavailable'
        )
      }
    } finally {
      operation.current.busy = false
      setSaving(false)
    }
  }

  const displayGroups = (names: string[]) =>
    names.length ? names.join(' · ') : t('No explicit group memberships')
  const notices = {
    unavailable: t('Access settings are unavailable. Reload to try again.'),
    conflict: t(
      'This user changed. Reload current access settings before saving.'
    ),
    unknown: t(
      'Save result is unknown. Check the original operation before continuing.'
    ),
  }
  if (!active) return null
  return (
    <>
      <Button
        type='button'
        size='sm'
        variant='ghost'
        onClick={() => changeOpen(true)}
      >
        {t('Access settings')}
      </Button>
      <Dialog open={open} onOpenChange={changeOpen}>
        <DialogContent className='max-h-[90dvh] overflow-y-auto sm:max-w-xl'>
          <DialogHeader>
            <DialogTitle>
              {t('Access settings')} · {userId}
            </DialogTitle>
            <DialogDescription>
              {t(
                'Manage service level, developer access and routing groups for this user.'
              )}
            </DialogDescription>
          </DialogHeader>
          {notice && (
            <Alert role='alert'>
              <AlertDescription>{notices[notice]}</AlertDescription>
            </Alert>
          )}
          {!snapshot && !notice && <p role='status'>{t('Loading')}</p>}
          {!snapshot && notice && (
            <Button
              type='button'
              variant='outline'
              onClick={() => {
                resetDraft()
                setReload((value) => value + 1)
              }}
            >
              {t('Reload access settings')}
            </Button>
          )}
          {snapshot && (
            <form onSubmit={submit} className='space-y-6'>
              {!snapshot.account_active && (
                <Alert role='alert'>
                  <AlertDescription>
                    {t(
                      'This account is closed. Access settings cannot be changed.'
                    )}
                  </AlertDescription>
                </Alert>
              )}
              {review ? (
                <section
                  aria-label={t('Review access changes')}
                  className='space-y-4'
                >
                  <h3 className='font-medium'>{t('Review access changes')}</h3>
                  <div className='grid grid-cols-[minmax(0,1fr)_minmax(0,1fr)_minmax(0,1fr)] gap-x-4 gap-y-3 text-sm'>
                    <span />
                    <span className='text-muted-foreground'>
                      {t('Current')}
                    </span>
                    <span className='font-medium'>{t('After change')}</span>
                    <span>{t('Service level')}</span>
                    <span>{snapshot.service_level}</span>
                    <span className='font-medium'>{review.service_level}</span>
                    <span>{t('Developer access')}</span>
                    <span>
                      {snapshot.developer_access_granted
                        ? t('Granted')
                        : t('Not granted')}
                    </span>
                    <span className='font-medium'>
                      {review.developer_access_granted
                        ? t('Granted')
                        : t('Not granted')}
                    </span>
                    <span>{t('Routing groups')}</span>
                    <span className='break-all'>
                      {displayGroups(snapshot.groups)}
                    </span>
                    <span className='font-medium break-all'>
                      {displayGroups(review.groups)}
                    </span>
                  </div>
                  <div className='text-sm'>
                    <p className='text-muted-foreground'>{t('Reason')}</p>
                    <p className='break-words'>{reason}</p>
                  </div>
                </section>
              ) : (
                <>
                  <div className='space-y-2'>
                    <Label htmlFor={`access-level-${userId}`}>
                      {t('Service level')}
                    </Label>
                    <select
                      id={`access-level-${userId}`}
                      className='border-input bg-background min-h-11 w-full rounded-md border px-3 py-2 text-sm'
                      value={level}
                      disabled={!snapshot.account_active}
                      onChange={(event) => setLevel(Number(event.target.value))}
                    >
                      {[0, 1, 2, 3, 4].map((value) => (
                        <option key={value} value={value}>
                          {value}
                        </option>
                      ))}
                    </select>
                  </div>
                  <div className='flex items-start gap-3'>
                    <Checkbox
                      id={`access-developer-${userId}`}
                      checked={developer}
                      disabled={!snapshot.account_active}
                      onCheckedChange={(value) => setDeveloper(value === true)}
                    />
                    <Label
                      htmlFor={`access-developer-${userId}`}
                      className='leading-5'
                    >
                      {t('Developer access')}
                    </Label>
                  </div>
                  <div className='space-y-2'>
                    <Label htmlFor={`access-groups-${userId}`}>
                      {t('Routing groups')}
                    </Label>
                    <Textarea
                      id={`access-groups-${userId}`}
                      className='min-h-28 font-mono'
                      value={groups}
                      disabled={!snapshot.account_active}
                      onChange={(event) => setGroups(event.target.value)}
                      aria-describedby={`access-groups-help-${userId}`}
                    />
                    <p
                      id={`access-groups-help-${userId}`}
                      className='text-muted-foreground text-xs'
                    >
                      {t(
                        'One group per line, up to 100. Names are exact; auto is reserved. Leave empty to remove explicit memberships.'
                      )}
                    </p>
                    {!userAccessValuesSchema.safeParse(values).success && (
                      <p role='alert' className='text-destructive text-sm'>
                        {t(
                          'Use unique group names of 1–64 characters, without control characters or auto.'
                        )}
                      </p>
                    )}
                  </div>
                  <div className='space-y-2'>
                    <Label htmlFor={`access-reason-${userId}`}>
                      {t('Reason')}
                    </Label>
                    <Input
                      id={`access-reason-${userId}`}
                      value={reason}
                      disabled={!snapshot.account_active}
                      onChange={(event) => setReason(event.target.value)}
                      maxLength={1000}
                      required
                    />
                    {reason && !validAccessReason(reason) && (
                      <p role='alert' className='text-destructive text-sm'>
                        {t(
                          'Enter a reason within 1,000 UTF-8 bytes, without control characters.'
                        )}
                      </p>
                    )}
                  </div>
                </>
              )}
              <p className='text-muted-foreground text-sm'>
                {t(
                  'Changing access revokes existing sessions and API keys. The user must sign in again and create new keys.'
                )}
              </p>
              <DialogFooter>
                {review && (
                  <Button
                    type='button'
                    variant='ghost'
                    disabled={saving}
                    onClick={() => setReview(undefined)}
                  >
                    {t('Back to editing')}
                  </Button>
                )}
                <Button
                  type='button'
                  variant='outline'
                  disabled={saving}
                  onClick={() => changeOpen(false)}
                >
                  {t('Cancel')}
                </Button>
                <Button
                  type='submit'
                  disabled={
                    !snapshot.account_active || !valid || !changed || saving
                  }
                >
                  {saving
                    ? t('Saving...')
                    : review
                      ? t('Confirm changes')
                      : t('Review changes')}
                </Button>
              </DialogFooter>
            </form>
          )}
        </DialogContent>
      </Dialog>
    </>
  )
}
