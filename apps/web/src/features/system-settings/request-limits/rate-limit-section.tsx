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
/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { ErrorState } from '@/components/error-state'
import { SectionPageLayout } from '@/components/layout/components/section-page-layout'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Switch } from '@/components/ui/switch'
import { Textarea } from '@/components/ui/textarea'
import { ROLE } from '@/lib/roles'
import { useAuthStore } from '@/stores/auth-store'

import {
  SettingsFormGrid,
  SettingsFormGridItem,
} from '../components/settings-form-layout'
import {
  configurationErrorKey,
  getRequestLimits,
  requestLimitPolicySchema,
  saveRequestLimits,
  type RequestLimitSnapshot,
  type SettingsScope,
} from './native-api'

function draftOf(snapshot: RequestLimitSnapshot) {
  return {
    enabled: snapshot.policy.enabled,
    window: String(snapshot.policy.window_seconds),
    user:
      snapshot.policy.user_limit === null
        ? ''
        : String(snapshot.policy.user_limit),
    groups: JSON.stringify(snapshot.policy.group_limits, null, 2),
  }
}

export function RequestLimitEditor({
  initial,
  scope,
  onSnapshot,
}: {
  initial: RequestLimitSnapshot
  scope: SettingsScope
  onSnapshot: (snapshot: RequestLimitSnapshot) => void
}) {
  const { t } = useTranslation()
  const [baseline, setBaseline] = useState(initial)
  const [draft, setDraft] = useState(() => draftOf(initial))
  const [invalid, setInvalid] = useState(false)
  const accept = (snapshot: RequestLimitSnapshot) => {
    setBaseline(snapshot)
    setDraft(draftOf(snapshot))
    setInvalid(false)
    onSnapshot(snapshot)
  }
  const save = useMutation({
    mutationFn: (policy: RequestLimitSnapshot['policy']) =>
      saveRequestLimits(scope, baseline.revision, policy),
    onSuccess: accept,
  })
  const reload = useMutation({
    mutationFn: () => getRequestLimits(scope),
    onSuccess: (snapshot) => {
      accept(snapshot)
      save.reset()
    },
  })
  const busy = save.isPending || reload.isPending
  const dirty = JSON.stringify(draft) !== JSON.stringify(draftOf(baseline))

  return (
    <form
      className='space-y-5'
      onSubmit={(event) => {
        event.preventDefault()
        if (busy) return
        let groupLimits: unknown
        try {
          groupLimits = JSON.parse(draft.groups)
        } catch {
          setInvalid(true)
          return
        }
        const parsed = requestLimitPolicySchema.safeParse({
          enabled: draft.enabled,
          window_seconds: Number(draft.window),
          user_limit: draft.user === '' ? null : Number(draft.user),
          group_limits: groupLimits,
        })
        if (!parsed.success) {
          setInvalid(true)
          return
        }
        setInvalid(false)
        save.mutate(parsed.data)
      }}
    >
      <p className='text-muted-foreground text-sm'>
        {t(
          'Admitted requests count before billing or upstream calls, including later failures. All API keys of a user share one limit; each group shares one limit across users.'
        )}
      </p>
      <fieldset disabled={busy} className='space-y-5'>
        <div className='flex items-center justify-between gap-4'>
          <Label htmlFor='native-rate-enabled'>
            {t('Enable Rate Limiting')}
          </Label>
          <Switch
            id='native-rate-enabled'
            checked={draft.enabled}
            onCheckedChange={(enabled) => setDraft({ ...draft, enabled })}
          />
        </div>
        <SettingsFormGrid>
          <div className='space-y-2'>
            <Label htmlFor='native-rate-window'>
              {t('Fixed window (seconds)')}
            </Label>
            <Input
              id='native-rate-window'
              type='number'
              min={1}
              max={86400}
              step={1}
              value={draft.window}
              onChange={(event) =>
                setDraft({ ...draft, window: event.target.value })
              }
            />
          </div>
          <div className='space-y-2'>
            <Label htmlFor='native-rate-user'>
              {t('Requests per user per window')}
            </Label>
            <Input
              id='native-rate-user'
              type='number'
              min={1}
              max={100_000_000}
              step={1}
              value={draft.user}
              onChange={(event) =>
                setDraft({ ...draft, user: event.target.value })
              }
              aria-describedby='native-rate-user-help'
            />
            <p
              id='native-rate-user-help'
              className='text-muted-foreground text-sm'
            >
              {t('Leave blank for no user limit.')}
            </p>
          </div>
          <SettingsFormGridItem span='full' className='space-y-2'>
            <Label htmlFor='native-rate-groups'>
              {t('Shared group request limits (JSON)')}
            </Label>
            <Textarea
              id='native-rate-groups'
              className='min-h-36 font-mono'
              value={draft.groups}
              onChange={(event) =>
                setDraft({ ...draft, groups: event.target.value })
              }
              placeholder='{"default": 300}'
              aria-describedby='native-rate-group-help'
            />
            <p
              id='native-rate-group-help'
              className='text-muted-foreground text-sm'
            >
              {t(
                'Map exact group names to request counts per window. Omitted groups have no group limit.'
              )}
            </p>
          </SettingsFormGridItem>
        </SettingsFormGrid>
      </fieldset>
      <p className='text-muted-foreground text-sm'>
        {t(
          'Windows follow the database clock. Traffic near a boundary can use two windows in quick succession. Rejected requests return HTTP 429 and Retry-After; changing the window uses counters for the new duration.'
        )}
      </p>
      {invalid && (
        <p role='alert' className='text-destructive text-sm'>
          {t(
            'Enter a window of 1–86400 seconds and positive integer limits up to 100000000. Enable at least one limit when rate limiting is on.'
          )}
        </p>
      )}
      {(save.error || reload.error) && (
        <p role='alert' className='text-destructive text-sm'>
          {t(
            reload.error
              ? 'Failed to load settings'
              : configurationErrorKey(save.error)
          )}
        </p>
      )}
      {save.isSuccess && !dirty && (
        <p role='status' className='text-muted-foreground text-sm'>
          {t('Setting updated successfully')}
        </p>
      )}
      <div className='flex flex-wrap gap-2'>
        <Button type='submit' disabled={busy || !dirty}>
          {save.isPending ? t('Saving...') : t('Save rate limits')}
        </Button>
        <Button
          type='button'
          variant='outline'
          disabled={busy}
          onClick={() => reload.mutate()}
        >
          {t('Reload current configuration')}
        </Button>
      </div>
    </form>
  )
}

function RequestLimitsWorkspace({ scope }: { scope: SettingsScope }) {
  const { t } = useTranslation()
  const client = useQueryClient()
  const key = ['native-request-limits', scope.userId, scope.sessionId]
  const query = useQuery({
    queryKey: key,
    queryFn: () => getRequestLimits(scope),
    retry: false,
    refetchOnWindowFocus: false,
  })
  if (query.isPending) return <p role='status'>{t('Loading...')}</p>
  if (query.isError) {
    return (
      <ErrorState
        title={t('Failed to load settings')}
        onRetry={() => void query.refetch()}
      />
    )
  }
  return (
    <RequestLimitEditor
      initial={query.data}
      scope={scope}
      onSnapshot={(snapshot) => client.setQueryData(key, snapshot)}
    />
  )
}

export function RateLimitSection() {
  const { t } = useTranslation()
  const user = useAuthStore((state) => state.auth.user)
  const sessionID = useAuthStore((state) => state.auth.session?.sid)
  return (
    <SectionPageLayout>
      <SectionPageLayout.Title>{t('Rate Limiting')}</SectionPageLayout.Title>
      <SectionPageLayout.Content>
        {user?.role === ROLE.SUPER_ADMIN && sessionID ? (
          <RequestLimitsWorkspace
            key={`${user.id}:${sessionID}`}
            scope={{ userId: user.id, sessionId: sessionID }}
          />
        ) : (
          <p role='alert'>{t('Super administrator access required.')}</p>
        )}
      </SectionPageLayout.Content>
    </SectionPageLayout>
  )
}
