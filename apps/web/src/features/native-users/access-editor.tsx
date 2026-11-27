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
import { readUserGroupOptions, updateUserAccess } from './access-api'
import {
  parseAccessGroups,
  sameAccess,
  userAccessValuesSchema,
  validAccessReason,
  type GroupOptions,
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
  const [directory, setDirectory] = useState<GroupOptions>()
  const snapshot = directory?.access
  const [groups, setGroups] = useState<string[]>([])
  const [search, setSearch] = useState('')
  const [limit, setLimit] = useState(50)
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
    void readUserGroupOptions(captured, userId, controller.signal).then(
      (value) => {
        if (generation !== holder.generation || controller.signal.aborted) {
          return
        }
        setDirectory(value)
        setGroups(value.access.groups)
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
    setDirectory(undefined)
    setReview(undefined)
    setReason('')
    setSearch('')
    setLimit(50)
    setNotice('')
  }

  function changeOpen(next: boolean) {
    if (operation.current.busy) return
    resetDraft()
    setOpen(next)
  }

  const values = {
    groups: parseAccessGroups(groups.join('\n')),
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
      setDirectory(undefined)
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
  const choices = [
    ...new Set([...(snapshot?.groups ?? []), ...(directory?.available ?? [])]),
  ].filter((name) => name.toLowerCase().includes(search.toLowerCase()))
  const groupNames = (names: string[]) =>
    names.length ? names.join(' · ') : t('None')
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
              {t('Manage routing groups for this user.')}
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
                  <dl className='grid grid-cols-[auto_1fr] gap-x-4 gap-y-2 text-sm'>
                    <dt>{t('Added groups')}</dt>
                    <dd>
                      {groupNames(
                        review.groups.filter(
                          (g) => !snapshot.groups.includes(g)
                        )
                      )}
                    </dd>
                    <dt>{t('Removed groups')}</dt>
                    <dd>
                      {groupNames(
                        snapshot.groups.filter(
                          (g) => !review.groups.includes(g)
                        )
                      )}
                    </dd>
                    <dt>{t('Effective groups after change')}</dt>
                    <dd>
                      {groupNames(
                        review.groups.filter((g) =>
                          directory!.available.includes(g)
                        )
                      )}
                    </dd>
                  </dl>
                  <div className='text-sm'>
                    <p className='text-muted-foreground'>{t('Reason')}</p>
                    <p className='break-words'>{reason}</p>
                  </div>
                </section>
              ) : (
                <>
                  <fieldset
                    className='space-y-2'
                    disabled={!snapshot.account_active}
                  >
                    <legend className='text-sm font-medium'>
                      {t('Explicit group memberships')}
                    </legend>
                    <dl className='grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm'>
                      <dt>{t('Current')}</dt>
                      <dd>{displayGroups(snapshot.groups)}</dd>
                      <dt>{t('Inherited groups')}</dt>
                      <dd>{groupNames(directory!.inherited)}</dd>
                      <dt>{t('Effective routing groups')}</dt>
                      <dd>{groupNames(directory!.effective)}</dd>
                    </dl>
                    <Input
                      id={`access-groups-${userId}`}
                      type='search'
                      disabled={!snapshot.account_active}
                      aria-label={t('Search groups')}
                      value={search}
                      onChange={(event) => {
                        setSearch(event.target.value)
                        setLimit(50)
                      }}
                    />
                    <div className='max-h-56 space-y-1 overflow-y-auto'>
                      {choices.slice(0, limit).map((name) => (
                        <label
                          key={name}
                          className='flex items-center gap-2 py-1'
                        >
                          <input
                            type='checkbox'
                            checked={groups.includes(name)}
                            aria-label={name}
                            disabled={
                              !groups.includes(name) && groups.length >= 100
                            }
                            onChange={(event) =>
                              setGroups(
                                event.target.checked
                                  ? [...groups, name]
                                  : groups.filter((item) => item !== name)
                              )
                            }
                          />
                          <span className='break-all'>{name}</span>
                        </label>
                      ))}
                      {!choices.length && <p>{t('No results found.')}</p>}
                      {choices.length > limit && (
                        <Button
                          type='button'
                          variant='ghost'
                          onClick={() => setLimit(limit + 50)}
                        >
                          {t('Show more')}
                        </Button>
                      )}
                    </div>
                  </fieldset>
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
