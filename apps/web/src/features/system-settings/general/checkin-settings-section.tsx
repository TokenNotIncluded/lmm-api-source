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
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { isAxiosError } from 'axios'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { z } from 'zod'

import { ErrorState } from '@/components/error-state'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Switch } from '@/components/ui/switch'
import {
  isCurrentNativeUsersScope,
  nativeUsersOptions,
  readNativeUsersScope,
  type NativeUsersScope,
} from '@/features/native-users/access'
import { api } from '@/lib/api'
import { ROLE } from '@/lib/roles'
import { useAuthStore } from '@/stores/auth-store'

import { SettingsSection } from '../components/settings-section'

const amount = z
  .string()
  .refine(
    (value) =>
      /^(0|[1-9][0-9]*)$/.test(value) &&
      value.length <= 19 &&
      BigInt(value) <= 9223372036854775807n
  )
const settingsSchema = z.object({
  revision: z.number().int().nonnegative().max(Number.MAX_SAFE_INTEGER),
  enabled: z.boolean(),
  min_quota: amount,
  max_quota: amount,
  writable: z.boolean(),
  module_enabled: z.boolean(),
  funding_available: z.boolean(),
  funding_limit: amount.nullable(),
})
type Settings = z.infer<typeof settingsSchema>
type Values = Pick<Settings, 'enabled' | 'min_quota' | 'max_quota'>
type SaveInput = Values & { revision: number; operation_key: string }
const endpoint = '/api/option/checkin'
const valuesOf = ({ enabled, min_quota, max_quota }: Settings): Values => ({
  enabled,
  min_quota,
  max_quota,
})

async function request(
  scope: NativeUsersScope,
  input?: SaveInput,
  signal?: AbortSignal
): Promise<Settings> {
  if (!isCurrentNativeUsersScope(scope) || signal?.aborted) {
    throw new Error('Session changed')
  }
  const options = nativeUsersOptions(scope, signal)
  const response = input
    ? await api.put<unknown>(endpoint, input, options)
    : await api.get<unknown>(endpoint, options)
  if (!isCurrentNativeUsersScope(scope) || signal?.aborted) {
    throw new Error('Session changed')
  }
  return z
    .object({ success: z.literal(true), data: settingsSchema })
    .parse(response.data).data
}

export function CheckinSettingsSection() {
  const { t } = useTranslation()
  const auth = useAuthStore((state) => state.auth)
  const scope = readNativeUsersScope(auth)
  if (!scope || scope.role !== ROLE.SUPER_ADMIN) {
    return (
      <ErrorState
        title={t('Owner access required')}
        description={t('Only the platform owner can manage check-in rewards.')}
      />
    )
  }
  return <CheckinWorkspace key={scope.sessionId} scope={scope} />
}

function CheckinWorkspace({ scope }: { scope: NativeUsersScope }) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const queryKey = ['native-checkin-settings', scope.userId, scope.sessionId]
  const query = useQuery({
    queryKey,
    queryFn: ({ signal }) => request(scope, undefined, signal),
    retry: false,
    gcTime: 0,
    staleTime: Infinity,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
  })
  if (query.isPending) return <p role='status'>{t('Loading settings...')}</p>
  if (!query.data) {
    return (
      <ErrorState
        title={t('Failed to load settings')}
        description={t(
          'Check-in settings require the installed checkins module and its dedicated database.'
        )}
        onRetry={() => void query.refetch()}
      />
    )
  }
  return (
    <CheckinEditor
      initial={query.data}
      scope={scope}
      onSaved={(saved) => queryClient.setQueryData(queryKey, saved)}
    />
  )
}

function CheckinEditor({
  initial,
  scope,
  onSaved,
}: {
  initial: Settings
  scope: NativeUsersScope
  onSaved: (saved: Settings) => void
}) {
  const { t } = useTranslation()
  const [baseline, setBaseline] = useState(initial)
  const [values, setValues] = useState<Values>(() => valuesOf(initial))
  const [attempt, setAttempt] = useState<SaveInput | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const dirty = JSON.stringify(values) !== JSON.stringify(valuesOf(baseline))
  const valid =
    amount.safeParse(values.min_quota).success &&
    amount.safeParse(values.max_quota).success &&
    BigInt(values.min_quota) <= BigInt(values.max_quota) &&
    (!values.enabled ||
      baseline.funding_limit === null ||
      BigInt(values.max_quota) <= BigInt(baseline.funding_limit))
  const update = (patch: Partial<Values>) => {
    setValues((current) => ({ ...current, ...patch }))
    setAttempt(null)
    setError('')
  }
  const save = async () => {
    if (!valid || !baseline.writable || busy) return
    const input = attempt ?? {
      ...values,
      revision: baseline.revision,
      operation_key: crypto.randomUUID(),
    }
    setAttempt(input)
    setBusy(true)
    setError('')
    try {
      const saved = await request(scope, input)
      setBaseline(saved)
      setValues(valuesOf(saved))
      setAttempt(null)
      onSaved(saved)
      toast.success(t('Setting saved'))
    } catch (failure) {
      if (!isCurrentNativeUsersScope(scope)) return
      setError(
        isAxiosError(failure) && failure.response?.status === 409
          ? t('Settings changed on the server. Reload before saving.')
          : t('Failed to save settings. Your changes are retained.')
      )
    } finally {
      if (isCurrentNativeUsersScope(scope)) setBusy(false)
    }
  }
  const reload = async () => {
    setBusy(true)
    setError('')
    try {
      const fresh = await request(scope)
      setBaseline(fresh)
      setValues(valuesOf(fresh))
      setAttempt(null)
      onSaved(fresh)
    } catch {
      if (isCurrentNativeUsersScope(scope)) {
        setError(t('Failed to load settings'))
      }
    } finally {
      if (isCurrentNativeUsersScope(scope)) setBusy(false)
    }
  }
  return (
    <SettingsSection title={t('Check-in Settings')}>
      <form
        className='max-w-2xl space-y-6'
        onSubmit={(event) => {
          event.preventDefault()
          void save()
        }}
      >
        <p className='text-muted-foreground text-sm'>
          {t(
            'Set one daily reward range for every user. Amounts are stored exactly in credits.'
          )}
        </p>
        {!baseline.writable && (
          <p role='status' className='text-muted-foreground text-sm'>
            {t(
              'Online check-in settings require the host upgrade-checkin-settings command.'
            )}
          </p>
        )}
        <fieldset disabled={busy || !baseline.writable} className='space-y-6'>
          <div className='flex items-center justify-between gap-4'>
            <Label htmlFor='checkin-enabled'>
              {t('Enable check-in feature')}
            </Label>
            <Switch
              id='checkin-enabled'
              checked={values.enabled}
              onCheckedChange={(enabled) => update({ enabled })}
            />
          </div>
          <div className='grid gap-5 sm:grid-cols-2'>
            <div className='space-y-2'>
              <Label htmlFor='checkin-minimum'>
                {t('Minimum check-in quota')}
              </Label>
              <Input
                id='checkin-minimum'
                inputMode='numeric'
                value={values.min_quota}
                onChange={(event) => update({ min_quota: event.target.value })}
              />
            </div>
            <div className='space-y-2'>
              <Label htmlFor='checkin-maximum'>
                {t('Maximum check-in quota')}
              </Label>
              <Input
                id='checkin-maximum'
                inputMode='numeric'
                value={values.max_quota}
                onChange={(event) => update({ max_quota: event.target.value })}
              />
            </div>
          </div>
        </fieldset>
        <div className='text-muted-foreground space-y-2 text-sm'>
          {!baseline.module_enabled && (
            <p role='status'>
              {t(
                'The checkins module is disabled. Saved settings remain editable; enable the module before claiming rewards.'
              )}
            </p>
          )}
          <p>
            {baseline.funding_available
              ? t('Core daily ceiling for this account: {{amount}} credits', {
                  amount: baseline.funding_limit,
                })
              : t(
                  'Core funding is unavailable. Saved settings remain editable; rewards are unavailable until funding is configured.'
                )}
          </p>
          <p>
            {t(
              'Claiming also requires the checkins module and configured human verification. Saving does not issue a reward.'
            )}
          </p>
        </div>
        {!valid && (
          <p role='alert' className='text-destructive text-sm'>
            {t(
              'Enter whole credit amounts with the minimum no greater than the maximum and within the Core ceiling.'
            )}
          </p>
        )}
        {error && (
          <p role='alert' className='text-destructive text-sm'>
            {error}
          </p>
        )}
        <div className='flex flex-wrap gap-3'>
          <Button
            type='submit'
            disabled={busy || !baseline.writable || !dirty || !valid}
          >
            {busy ? t('Saving...') : t('Save check-in settings')}
          </Button>
          <Button
            type='button'
            variant='outline'
            disabled={busy}
            onClick={() => void reload()}
          >
            {dirty ? t('Discard changes and reload') : t('Refresh')}
          </Button>
        </div>
      </form>
    </SettingsSection>
  )
}
