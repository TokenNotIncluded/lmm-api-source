/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useRef, useState, type FormEvent } from 'react'
import { useTranslation } from 'react-i18next'

import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'

import { isCurrentNativeUsersScope, type NativeUsersScope } from './access'
import {
  createNativeUser,
  NativeUserCreationFailure,
  validNativeUserDraft,
} from './create-api'

export function NativeCreateUser({
  scope,
  onCreated,
  onAccessDenied,
}: {
  scope: NativeUsersScope
  onCreated: () => void
  onAccessDenied: () => void
}) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const [draft, setDraft] = useState({
    username: '',
    password: '',
    display_name: '',
  })
  const [phase, setPhase] = useState<'ready' | 'saving' | 'unknown'>('ready')
  const [notice, setNotice] = useState<
    'none' | 'success' | 'unknown' | 'conflict' | 'invalid'
  >('none')
  const notices = {
    none: '',
    success: t('User created successfully'),
    unknown: t(
      'User creation could not be confirmed. Check the user list before creating another account.'
    ),
    conflict: t('This username is already taken. Choose another username.'),
    invalid: t('Invalid user details'),
  }
  const busy = useRef(false)
  if (!isCurrentNativeUsersScope(scope)) return null
  const submit = async (event: FormEvent) => {
    event.preventDefault()
    if (busy.current || phase !== 'ready' || !validNativeUserDraft(draft))
      return
    busy.current = true
    setPhase('saving')
    setNotice('none')
    try {
      await createNativeUser(scope, draft)
      if (!isCurrentNativeUsersScope(scope)) return
      setDraft({ username: '', password: '', display_name: '' })
      setPhase('ready')
      setNotice('success')
      setOpen(false)
      onCreated()
    } catch (error) {
      if (!isCurrentNativeUsersScope(scope)) return
      const kind =
        error instanceof NativeUserCreationFailure ? error.kind : 'unknown'
      setDraft((previous) => ({ ...previous, password: '' }))
      if (kind === 'forbidden') {
        onAccessDenied()
        return
      }
      setPhase(kind === 'unknown' ? 'unknown' : 'ready')
      setNotice(
        kind === 'unknown'
          ? 'unknown'
          : kind === 'conflict'
            ? 'conflict'
            : 'invalid'
      )
    } finally {
      busy.current = false
    }
  }
  return (
    <section
      className='space-y-3 rounded-lg border p-4'
      aria-label={t('Create user')}
    >
      <Button
        type='button'
        variant='outline'
        disabled={phase === 'saving'}
        onClick={() => {
          setOpen((value) => !value)
          setDraft((previous) => ({ ...previous, password: '' }))
        }}
      >
        {open ? t('Cancel') : t('Create user')}
      </Button>
      {notice !== 'none' && (
        <Alert role='status'>
          <AlertDescription>{notices[notice]}</AlertDescription>
        </Alert>
      )}
      {open && (
        <form
          onSubmit={submit}
          className='grid gap-4 sm:grid-cols-2'
          aria-busy={phase === 'saving'}
          autoComplete='off'
        >
          <div className='space-y-2'>
            <Label htmlFor='native-create-username'>{t('Username')}</Label>
            <Input
              id='native-create-username'
              value={draft.username}
              autoComplete='off'
              disabled={phase !== 'ready'}
              onChange={(event) =>
                setDraft({ ...draft, username: event.target.value })
              }
              required
            />
          </div>
          <div className='space-y-2'>
            <Label htmlFor='native-create-display-name'>
              {t('Display name')}
            </Label>
            <Input
              id='native-create-display-name'
              value={draft.display_name}
              disabled={phase !== 'ready'}
              onChange={(event) =>
                setDraft({ ...draft, display_name: event.target.value })
              }
            />
          </div>
          <div className='space-y-2'>
            <Label htmlFor='native-create-password'>{t('Password')}</Label>
            <Input
              id='native-create-password'
              type='password'
              autoComplete='new-password'
              placeholder={t('Enter password (8-20 characters)')}
              value={draft.password}
              disabled={phase !== 'ready'}
              onChange={(event) =>
                setDraft({ ...draft, password: event.target.value })
              }
              required
            />
          </div>
          <div className='flex items-end'>
            <Button
              type='submit'
              disabled={phase !== 'ready' || !validNativeUserDraft(draft)}
            >
              {phase === 'saving' ? t('Creating...') : t('Create user')}
            </Button>
          </div>
        </form>
      )}
    </section>
  )
}
