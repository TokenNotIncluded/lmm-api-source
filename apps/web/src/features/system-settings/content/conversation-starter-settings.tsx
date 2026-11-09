/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { ErrorState } from '@/components/error-state'
import { LoadingState } from '@/components/loading-state'
import { Button } from '@/components/ui/button'
import { ROLE } from '@/lib/roles'
import { useAuthStore } from '@/stores/auth-store'

import { FormNavigationGuard } from '../components/form-navigation-guard'
import { SettingsPageFormActions } from '../components/settings-page-context'
import {
  getAssistantPresetSettings,
  presetSettingsConflict,
  saveAssistantPresetSettings,
  validPresetConfiguration,
  type AssistantPresetSettings,
} from './assistant-preset-settings-api'
import { ConversationStartersEditor } from './conversation-starters-editor'

type PresetIdentity = {
  userId: number
  sessionId: string
  role: number
  embedded?: boolean
}

export function ConversationStarterSettings({
  embedded = false,
}: {
  embedded?: boolean
}) {
  const auth = useAuthStore((state) => state.auth)
  const { t } = useTranslation()
  if (!auth.user || !auth.session || auth.user.role < ROLE.ADMIN) {
    return embedded ? (
      <p role='alert' className='text-muted-foreground text-sm'>
        {t('Access Forbidden')}
      </p>
    ) : (
      <ErrorState title={t('Access Forbidden')} />
    )
  }
  return (
    <PresetSettingsLoader
      key={`${auth.user.id}:${auth.session.sid}:${auth.user.role}`}
      userId={auth.user.id}
      sessionId={auth.session.sid}
      role={auth.user.role}
      embedded={embedded}
    />
  )
}

function PresetSettingsLoader(identity: PresetIdentity) {
  const { t } = useTranslation()
  const queryKey = [
    'assistant-preset-settings',
    identity.userId,
    identity.sessionId,
    identity.role,
  ]
  const settings = useQuery({
    queryKey,
    queryFn: ({ signal }) =>
      getAssistantPresetSettings({ signal, authScope: identity }),
    retry: false,
    refetchOnWindowFocus: false,
  })
  if (!settings.data) {
    return settings.isError ? (
      <ErrorState
        title={t('Unable to load settings')}
        onRetry={() => void settings.refetch()}
      />
    ) : (
      <LoadingState message={t('Loading settings...')} />
    )
  }
  return (
    <PresetSettingsEditor
      {...identity}
      initial={settings.data}
      queryKey={queryKey}
      reload={async () => {
        const response = await settings.refetch()
        if (response.error || !response.data) {
          throw response.error ?? new Error('Settings unavailable')
        }
        return response.data
      }}
    />
  )
}

function PresetSettingsEditor({
  initial,
  queryKey,
  reload,
  ...identity
}: PresetIdentity & {
  initial: AssistantPresetSettings
  queryKey: (string | number)[]
  reload: () => Promise<AssistantPresetSettings>
}) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const [baseline, setBaseline] = useState(initial)
  const [draft, setDraft] = useState(initial.configuration)
  const [error, setError] = useState<string | null>(null)
  const [conflict, setConflict] = useState(false)
  const [refreshing, setRefreshing] = useState(false)
  const [reviewedVersion, setReviewedVersion] = useState(false)
  const readOnly = identity.role !== ROLE.SUPER_ADMIN
  const dirty = draft !== baseline.configuration
  const current = () => {
    const auth = useAuthStore.getState().auth
    return (
      auth.user?.id === identity.userId &&
      auth.session?.sid === identity.sessionId &&
      auth.user.role === identity.role
    )
  }
  const save = useMutation({
    mutationFn: (configuration: string) =>
      saveAssistantPresetSettings(
        { configuration, revision: baseline.revision },
        { authScope: identity }
      ),
    retry: false,
    onSuccess: (saved, submitted) => {
      if (!current()) return
      setBaseline(saved)
      setDraft((latest) =>
        latest === submitted ? saved.configuration : latest
      )
      setError(null)
      setConflict(false)
      setReviewedVersion(false)
      queryClient.setQueryData(queryKey, saved)
      void queryClient.invalidateQueries({
        queryKey: ['assistant-pre-conversation-presets'],
      })
      toast.success(t('Settings saved'))
    },
    onError: (error) => {
      if (!current()) return
      const conflict = presetSettingsConflict(error)
      setConflict(conflict)
      setError(
        conflict
          ? 'Settings changed. Reload the current configuration before saving again.'
          : 'Save failed'
      )
    },
  })
  const busy = save.isPending || refreshing
  const submit = () => {
    if (readOnly || !current() || busy || conflict || !dirty) return
    if (!validPresetConfiguration(draft)) {
      setError(
        'Configuration or selection is invalid. Check the fields and limits.'
      )
      return
    }
    save.mutate(draft)
  }
  const reset = () => {
    setDraft(baseline.configuration)
    setError(null)
    setConflict(false)
  }
  const refresh = async () => {
    setRefreshing(true)
    try {
      const latest = await reload()
      if (!current()) return
      setBaseline(latest)
      setConflict(false)
      setError(null)
      setReviewedVersion(true)
      // The draft is deliberately kept. Only another explicit Save can
      // publish it against the newly reviewed revision.
    } catch {
      if (current()) setError('Unable to refresh settings')
    } finally {
      setRefreshing(false)
    }
  }
  return (
    <div className='space-y-5' data-testid='assistant-preset-settings'>
      <FormNavigationGuard when={dirty} />
      {!identity.embedded && (
        <SettingsPageFormActions
          onSave={submit}
          onReset={reset}
          isSaving={busy}
          isSaveDisabled={readOnly || conflict || !dirty}
          isResetDisabled={!dirty || busy}
          saveLabel='Save Settings'
        />
      )}
      <div className='space-y-2'>
        <h3 className='text-base font-medium'>
          {t('Conversation starter prompts')}
        </h3>
        <p className='text-muted-foreground text-sm'>
          {t(
            'Optional JSON list of starter buttons. Leave empty for reviewed defaults; use [] to hide them.'
          )}
        </p>
        {readOnly && (
          <p className='text-muted-foreground text-sm'>{t('Read only')}</p>
        )}
      </div>
      {error && (
        <div role='alert' className='space-y-2 text-sm'>
          <p>{t(error)}</p>
          {conflict && (
            <Button
              type='button'
              variant='outline'
              size='sm'
              disabled={busy}
              onClick={() => void refresh()}
            >
              {t('Keep draft and reload revision')}
            </Button>
          )}
        </div>
      )}
      {reviewedVersion && (
        <details className='space-y-2 text-sm' open>
          <summary>
            {t('Current version')} · {baseline.revision}
          </summary>
          <pre className='max-h-48 overflow-auto text-xs break-words whitespace-pre-wrap'>
            {baseline.configuration || t('Restore defaults')}
          </pre>
        </details>
      )}
      <ConversationStartersEditor
        value={draft}
        disabled={readOnly || busy}
        onChange={setDraft}
      />
      <div className='flex flex-wrap justify-end gap-2'>
        <Button
          type='button'
          variant='outline'
          onClick={reset}
          disabled={!dirty || busy}
        >
          {t('Reset')}
        </Button>
        <Button
          type='button'
          onClick={submit}
          disabled={readOnly || conflict || !dirty || busy}
        >
          {t(busy ? 'Saving...' : 'Save Settings')}
        </Button>
      </div>
    </div>
  )
}
