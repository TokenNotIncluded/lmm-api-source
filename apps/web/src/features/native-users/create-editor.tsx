/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { Plus } from 'lucide-react'
import { useRef, useState, type FormEvent } from 'react'
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
  DialogTrigger,
} from '@/components/ui/dialog'
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
  onCreated: (userId: string) => void
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
    'none' | 'unknown' | 'conflict' | 'invalid'
  >('none')
  const notices = {
    none: '',
    unknown: t(
      'User creation could not be confirmed. Check the user list before creating another account.'
    ),
    conflict: t('This username is already taken. Choose another username.'),
    invalid: t('Invalid user details'),
  }
  const busy = useRef(false)
  if (!isCurrentNativeUsersScope(scope)) return null

  const changeOpen = (next: boolean) => {
    if (busy.current) return
    setOpen(next)
    setDraft((previous) => ({ ...previous, password: '' }))
  }
  const submit = async (event: FormEvent) => {
    event.preventDefault()
    if (busy.current || phase !== 'ready' || !validNativeUserDraft(draft)) {
      return
    }
    busy.current = true
    setPhase('saving')
    setNotice('none')
    try {
      const created = await createNativeUser(scope, draft)
      if (!isCurrentNativeUsersScope(scope)) return
      setDraft({ username: '', password: '', display_name: '' })
      setPhase('ready')
      setOpen(false)
      toast.success(t('User created successfully'))
      onCreated(created)
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
    <Dialog open={open} onOpenChange={changeOpen}>
      <DialogTrigger render={<Button className='min-h-11' />}>
        <Plus aria-hidden='true' />
        {t('Create user')}
      </DialogTrigger>
      <DialogContent
        className='sm:max-w-lg'
        showCloseButton={phase !== 'saving'}
      >
        <DialogHeader>
          <DialogTitle>{t('Create user')}</DialogTitle>
          <DialogDescription>
            {t(
              'Create a personal account. Team membership and spending permissions are managed separately.'
            )}
          </DialogDescription>
        </DialogHeader>
        {notice !== 'none' && (
          <Alert role='alert'>
            <AlertDescription>{notices[notice]}</AlertDescription>
          </Alert>
        )}
        <form
          onSubmit={submit}
          className='space-y-5'
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
          <DialogFooter>
            <Button
              type='button'
              variant='ghost'
              disabled={phase === 'saving'}
              onClick={() => changeOpen(false)}
            >
              {t('Cancel')}
            </Button>
            <Button
              type='submit'
              disabled={phase !== 'ready' || !validNativeUserDraft(draft)}
            >
              {phase === 'saving' ? t('Creating...') : t('Create user')}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
