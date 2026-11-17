/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useEffect, useRef, useState, type FormEvent } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Skeleton } from '@/components/ui/skeleton'
import { useAuthStore } from '@/stores/auth-store'

import {
  currentProfileIdentity,
  saveProfileDisplayName,
  validProfileDisplayName,
} from '../identity-api'
import type { UserProfile } from '../types'

export function ProfileIdentityCard({
  profile,
  loading,
  onProfileUpdate,
}: {
  profile: UserProfile | null
  loading: boolean
  onProfileUpdate: () => void | Promise<void>
}) {
  const { t } = useTranslation()
  const userId = useAuthStore((state) => state.auth.user?.id)
  const sessionId = useAuthStore((state) => state.auth.session?.sid)
  const [displayName, setDisplayName] = useState(profile?.display_name ?? '')
  const [saving, setSaving] = useState(false)
  const [notice, setNotice] = useState<'saved' | 'failed' | 'invalid' | null>(
    null
  )
  const active = useRef<AbortController | null>(null)
  const available = !loading && profile?.id === userId && Boolean(sessionId)
  const changed = displayName !== (profile?.display_name ?? '')

  useEffect(() => {
    setDisplayName(profile?.display_name ?? '')
    setNotice(null)
  }, [profile?.display_name, userId, sessionId])

  useEffect(
    () => () => {
      active.current?.abort()
    },
    [userId, sessionId]
  )

  async function save(event: FormEvent) {
    event.preventDefault()
    if (!available || !userId || !sessionId || !changed || active.current)
      {return}
    if (!validProfileDisplayName(displayName)) {
      setNotice('invalid')
      return
    }
    const scope = { userId, sessionId }
    const request = new AbortController()
    active.current = request
    const unsubscribe = useAuthStore.subscribe(() => {
      if (!currentProfileIdentity(scope)) request.abort()
    })
    setSaving(true)
    setNotice(null)
    try {
      const updated = await saveProfileDisplayName(
        scope,
        displayName,
        request.signal
      )
      if (request.signal.aborted || !currentProfileIdentity(scope)) return
      const auth = useAuthStore.getState().auth
      if (auth.user)
        {auth.setUser({ ...auth.user, display_name: updated.display_name })}
      await onProfileUpdate()
      if (!request.signal.aborted && currentProfileIdentity(scope))
        {setNotice('saved')}
    } catch {
      if (!request.signal.aborted && currentProfileIdentity(scope))
        {setNotice('failed')}
    } finally {
      unsubscribe()
      if (active.current === request) {
        active.current = null
        setSaving(false)
      }
    }
  }

  return (
    <section
      aria-labelledby='profile-identity-title'
      className='bg-card rounded-2xl p-5 sm:p-7'
    >
      <header className='mb-6 space-y-1'>
        <h2 id='profile-identity-title' className='text-base font-semibold'>
          {t('Your identity')}
        </h2>
        <p className='text-muted-foreground text-sm'>
          {t('Choose how your name appears across the console.')}
        </p>
      </header>
      {loading ? (
        <Skeleton className='h-32 w-full' />
      ) : (
        <form onSubmit={save} className='space-y-5'>
          <div className='grid gap-5 sm:grid-cols-2'>
            <div className='space-y-2'>
              <Label htmlFor='profile-display-name-input'>
                {t('Display Name')}
              </Label>
              <Input
                id='profile-display-name-input'
                value={available ? displayName : ''}
                onChange={(event) => {
                  setDisplayName(event.target.value)
                  setNotice(null)
                }}
                disabled={!available || saving}
                maxLength={256}
                autoComplete='nickname'
                aria-describedby='profile-display-name-help'
                className='min-h-11'
              />
              <p
                id='profile-display-name-help'
                className='text-muted-foreground text-xs leading-relaxed'
              >
                {t(
                  'Leave this empty to use your username. Up to 256 UTF-8 bytes.'
                )}
              </p>
            </div>
            <div className='space-y-2'>
              <Label htmlFor='profile-username'>{t('Username')}</Label>
              <Input
                id='profile-username'
                value={profile?.username ?? ''}
                readOnly
                className='text-muted-foreground min-h-11'
              />
              <p className='text-muted-foreground text-xs'>
                {t('Your username is used to sign in.')}
              </p>
            </div>
          </div>
          {notice && (
            <p
              role={notice === 'saved' ? 'status' : 'alert'}
              className={
                notice === 'saved'
                  ? 'text-success text-sm'
                  : 'text-destructive text-sm'
              }
            >
              {notice === 'saved'
                ? t('Profile updated successfully')
                : notice === 'invalid'
                  ? t(
                      'Use a display name within 256 UTF-8 bytes, without control characters.'
                    )
                  : t(
                      'Could not confirm the profile update. Reload your profile and try again.'
                    )}
            </p>
          )}
          <div className='flex flex-wrap items-center gap-2'>
            <Button type='submit' disabled={!available || !changed || saving}>
              {saving ? t('Saving...') : t('Save changes')}
            </Button>
            <Button
              type='button'
              variant='ghost'
              disabled={!available || saving || (!changed && !notice)}
              onClick={() => {
                setDisplayName(profile?.display_name ?? '')
                setNotice(null)
              }}
            >
              {t('Cancel')}
            </Button>
          </div>
        </form>
      )}
    </section>
  )
}
