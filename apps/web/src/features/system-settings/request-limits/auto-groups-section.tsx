/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { ErrorState } from '@/components/error-state'
import { Button } from '@/components/ui/button'
import { Label } from '@/components/ui/label'
import { Textarea } from '@/components/ui/textarea'
import { ROLE } from '@/lib/roles'
import { useAuthStore } from '@/stores/auth-store'

import {
  autoGroupOrderSchema,
  configurationErrorKey,
  getAutoGroupOrder,
  saveAutoGroupOrder,
  type AutoGroupSnapshot,
  type SettingsScope,
} from './native-api'

export function AutoGroupEditor({
  initial,
  scope,
  onSnapshot,
}: {
  initial: AutoGroupSnapshot
  scope: SettingsScope
  onSnapshot: (snapshot: AutoGroupSnapshot) => void
}) {
  const { t } = useTranslation()
  const [baseline, setBaseline] = useState(initial)
  const [draft, setDraft] = useState(() =>
    JSON.stringify(initial.groups, null, 2)
  )
  const [invalid, setInvalid] = useState(false)
  const accept = (snapshot: AutoGroupSnapshot) => {
    setBaseline(snapshot)
    setDraft(JSON.stringify(snapshot.groups, null, 2))
    setInvalid(false)
    onSnapshot(snapshot)
  }
  const save = useMutation({
    mutationFn: (groups: string[]) =>
      saveAutoGroupOrder(scope, baseline.revision, groups),
    onSuccess: accept,
  })
  const reload = useMutation({
    mutationFn: () => getAutoGroupOrder(scope),
    onSuccess: (snapshot) => {
      accept(snapshot)
      save.reset()
    },
  })
  const busy = save.isPending || reload.isPending
  const dirty = draft !== JSON.stringify(baseline.groups, null, 2)
  return (
    <form
      className='space-y-3'
      onSubmit={(event) => {
        event.preventDefault()
        if (busy) return
        let groups: unknown
        try {
          groups = JSON.parse(draft)
        } catch {
          setInvalid(true)
          return
        }
        const parsed = autoGroupOrderSchema.safeParse(groups)
        if (!parsed.success) {
          setInvalid(true)
          return
        }
        setInvalid(false)
        save.mutate(parsed.data)
      }}
    >
      <Label htmlFor='native-auto-order'>{t('Global Auto group order')}</Label>
      <Textarea
        id='native-auto-order'
        className='min-h-32 font-mono'
        value={draft}
        onChange={(event) => setDraft(event.target.value)}
        disabled={busy}
        aria-describedby='native-auto-help'
      />
      <p id='native-auto-help' className='text-muted-foreground text-sm'>
        {t(
          'Enter an ordered JSON array of exact group names. An empty array uses all authorized, enabled groups in name order. Inherited API keys follow changes immediately; custom lists stay unchanged.'
        )}
      </p>
      <p className='text-muted-foreground text-sm'>
        {t(
          'This order grants no access. Runtime routing filters unavailable or unauthorized groups. Custom API key lists allow up to 16 groups; inheritance is not truncated.'
        )}
      </p>
      {invalid && (
        <p role='alert' className='text-destructive text-sm'>
          {t('Enter a JSON array of unique, valid group names.')}
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
          {save.isPending ? t('Saving...') : t('Save Auto group order')}
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
function AutoGroupsWorkspace({ scope }: { scope: SettingsScope }) {
  const { t } = useTranslation()
  const client = useQueryClient()
  const key = ['native-auto-group-order', scope.userId, scope.sessionId]
  const query = useQuery({
    queryKey: key,
    queryFn: () => getAutoGroupOrder(scope),
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
    <AutoGroupEditor
      initial={query.data}
      scope={scope}
      onSnapshot={(snapshot) => {
        client.setQueryData(key, snapshot)
        void client.invalidateQueries({ queryKey: ['token-auto-groups'] })
      }}
    />
  )
}
export function AutoGroupsSection() {
  const { t } = useTranslation()
  const user = useAuthStore((state) => state.auth.user)
  const sessionID = useAuthStore((state) => state.auth.session?.sid)
  return (
    <section className='border-b pb-6'>
      {user?.role === ROLE.SUPER_ADMIN && sessionID ? (
        <AutoGroupsWorkspace
          key={`${user.id}:${sessionID}`}
          scope={{ userId: user.id, sessionId: sessionID }}
        />
      ) : (
        <p role='alert'>{t('Super administrator access required.')}</p>
      )}
    </section>
  )
}
