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
import { useQuery } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { useEffect, useState, type FormEvent } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Switch } from '@/components/ui/switch'
import { useAuthStore } from '@/stores/auth-store'

import {
  notificationPreferencesSchema,
  readNotificationPreferences,
  saveNotificationPreferences,
} from '../../notification-preferences'
import type { UserProfile } from '../../types'

interface NotificationTabProps {
  profile: UserProfile | null
  onUpdate: () => void
}

export function NotificationTab({ onUpdate }: NotificationTabProps) {
  const { t } = useTranslation()
  const userId = useAuthStore((state) => state.auth.user?.id)
  const sessionId = useAuthStore((state) => state.auth.session?.sid)
  const scope = { userId, sessionId }
  const preferences = useQuery({
    queryKey: ['core-notification-preferences', userId, sessionId],
    queryFn: ({ signal }) => readNotificationPreferences(scope, signal),
    enabled: Boolean(userId && sessionId),
    retry: false,
    refetchOnWindowFocus: false,
  })
  const [enabled, setEnabled] = useState(true)
  const [threshold, setThreshold] = useState('0')
  const [busy, setBusy] = useState(false)
  const [notice, setNotice] = useState('')
  useEffect(() => {
    setNotice('')
    setEnabled(preferences.data?.enabled ?? true)
    setThreshold(preferences.data?.low_balance_units ?? '0')
  }, [preferences.data, userId, sessionId])

  async function save(event: FormEvent) {
    event.preventDefault()
    if (busy || !preferences.data || !userId || !sessionId) return
    const input = notificationPreferencesSchema.safeParse({
      ...preferences.data,
      enabled,
      low_balance_units: threshold,
    })
    if (!input.success) {
      setNotice(t('Invalid input'))
      return
    }
    setBusy(true)
    setNotice('')
    try {
      await saveNotificationPreferences(scope, input.data)
      const auth = useAuthStore.getState().auth
      if (auth.user?.id !== userId || auth.session?.sid !== sessionId) return
      await preferences.refetch()
      toast.success(t('Settings updated successfully'))
      onUpdate()
    } catch {
      const auth = useAuthStore.getState().auth
      if (auth.user?.id === userId && auth.session?.sid === sessionId) {
        setNotice(
          t('Notification settings may have changed. Reload and try again.')
        )
      }
    } finally {
      setBusy(false)
    }
  }

  if (preferences.isPending) {
    return (
      <p role='status' className='text-muted-foreground text-sm'>
        {t('Loading...')}
      </p>
    )
  }
  if (!preferences.data) {
    return (
      <Alert role='alert'>
        <AlertDescription className='space-y-3'>
          <p>{t('Unable to load notification settings')}</p>
          <Button
            type='button'
            variant='outline'
            disabled={preferences.isFetching}
            onClick={() => void preferences.refetch()}
          >
            {t('Retry')}
          </Button>
        </AlertDescription>
      </Alert>
    )
  }

  return (
    <form onSubmit={save} className='space-y-5'>
      <div className='flex items-center justify-between gap-4'>
        <div className='space-y-1'>
          <Label htmlFor='low-balance-alert-enabled'>
            {t('Low balance reminders')}
          </Label>
          <p className='text-muted-foreground text-sm'>
            {t(
              'Receive a notification in your inbox when your available balance falls below the threshold.'
            )}
          </p>
        </div>
        <Switch
          id='low-balance-alert-enabled'
          checked={enabled}
          onCheckedChange={setEnabled}
          disabled={busy}
        />
      </div>
      <div className='space-y-2'>
        <Label htmlFor='low-balance-threshold'>
          {t('Balance threshold (credit units)')}
        </Label>
        <Input
          id='low-balance-threshold'
          value={threshold}
          onChange={(event) => setThreshold(event.target.value)}
          inputMode='numeric'
          pattern='(0|[1-9][0-9]*)'
          maxLength={16}
          required
          disabled={busy || !enabled}
          aria-describedby='low-balance-threshold-help'
        />
        <p
          id='low-balance-threshold-help'
          className='text-muted-foreground text-xs'
        >
          {t(
            '500,000 credit units equal USD 1. Set zero to disable balance reminders.'
          )}
        </p>
      </div>
      {notice && (
        <Alert role='alert'>
          <AlertDescription>{notice}</AlertDescription>
        </Alert>
      )}
      <div className='flex flex-wrap items-center gap-2'>
        <Button type='submit' disabled={busy || preferences.isFetching}>
          {busy ? t('Saving...') : t('Save')}
        </Button>
        <Button
          type='button'
          variant='outline'
          disabled={busy || preferences.isFetching}
          onClick={() => void preferences.refetch()}
        >
          {t('Reload')}
        </Button>
        <Link
          to='/todos'
          className='text-primary rounded-sm px-2 text-sm underline underline-offset-4 focus-visible:outline-2'
        >
          {t('View notifications')}
        </Link>
      </div>
    </form>
  )
}
