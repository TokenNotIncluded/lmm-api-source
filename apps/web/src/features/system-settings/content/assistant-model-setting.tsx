/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { isAxiosError } from 'axios'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { z } from 'zod'

import { Button } from '@/components/ui/button'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import {
  isCurrentNativeUsersScope,
  type NativeUsersScope,
} from '@/features/native-users/access'
import { api } from '@/lib/api'

const path = '/api/option/assistant-model'
const version = z.object({
  routing: z.number().int().nonnegative(),
  pricing: z.number().int().nonnegative(),
})
const envelope = z.object({
  success: z.literal(true),
  data: z.object({
    model: z.string(),
    default_model: z.string(),
    group: z.string(),
    models: z.array(z.string().min(1)),
    version,
  }),
})
type Draft = { model: string; version: z.infer<typeof version> }

function denied(error: unknown) {
  return (
    isAxiosError(error) &&
    (error.response?.status === 401 || error.response?.status === 403)
  )
}

export function AssistantModelSetting({ scope }: { scope: NativeUsersScope }) {
  const { t } = useTranslation()
  const client = useQueryClient()
  const queryKey = [
    'assistant-model-setting',
    scope.userId,
    scope.sessionId,
    scope.role,
  ]
  const request = {
    authScope: scope,
    skipAuthRefresh: true,
    skipBusinessError: true,
    skipErrorHandler: true,
  }
  const read = useQuery({
    queryKey,
    retry: false,
    refetchOnWindowFocus: false,
    queryFn: async ({ signal }) =>
      envelope.parse(
        (await api.get<unknown>(path, { ...request, signal })).data
      ).data,
  })
  const [draft, setDraft] = useState<Draft>()
  const [state, setState] = useState<
    'idle' | 'saving' | 'saved' | 'failed' | 'conflict' | 'forbidden'
  >('idle')
  const forbidden = state === 'forbidden' || denied(read.error)
  const data = forbidden ? undefined : read.data
  const current = data?.model || data?.default_model || ''
  const options = [...new Set(data?.models ?? [])]
  const selected = draft?.model ?? current
  const conflict =
    state === 'conflict' ||
    Boolean(
      draft &&
      data &&
      (draft.version.routing !== data.version.routing ||
        draft.version.pricing !== data.version.pricing)
    )
  const canSave = Boolean(
    draft &&
    selected !== current &&
    options.includes(selected) &&
    !conflict &&
    !read.isFetching &&
    !read.isError &&
    state !== 'saving'
  )
  async function save() {
    if (!draft || !canSave || !isCurrentNativeUsersScope(scope)) return
    setState('saving')
    try {
      const response = await api.put<unknown>(
        path,
        { model: draft.model, expected_version: draft.version },
        request
      )
      const saved = envelope.parse(response.data).data
      if (!isCurrentNativeUsersScope(scope)) return
      if (saved.model !== draft.model) {
        throw new Error('Unexpected assistant model receipt')
      }
      client.setQueryData(queryKey, saved)
      setDraft(undefined)
      setState('saved')
      void client.invalidateQueries({
        queryKey: ['assistant-status', scope.userId, scope.sessionId],
      })
    } catch (error) {
      if (!isCurrentNativeUsersScope(scope)) return
      if (denied(error)) setState('forbidden')
      else if (isAxiosError(error) && error.response?.status === 409) {
        setState('conflict')
      } else setState('failed')
    }
  }
  async function reload() {
    const result = await read.refetch()
    if (result.isSuccess && isCurrentNativeUsersScope(scope)) {
      setDraft(undefined)
      setState('idle')
    }
  }
  return (
    <div className='space-y-2 border-t pt-3'>
      <Label htmlFor='assistant-host-model'>{t('Assistant model')}</Label>
      {data && (
        <p className='text-muted-foreground text-xs'>
          {t('Group')} · {data.group || t('Unknown')}
        </p>
      )}
      {data && (
        <div className='flex flex-wrap items-center gap-2'>
          <NativeSelect
            id='assistant-host-model'
            value={selected}
            disabled={
              options.length === 0 ||
              state === 'saving' ||
              conflict ||
              read.isError
            }
            onChange={(event) => {
              const model = event.target.value
              setDraft(
                model === current
                  ? undefined
                  : { model, version: draft?.version ?? data.version }
              )
              setState('idle')
            }}
          >
            {!options.includes(selected) && (
              <NativeSelectOption value={selected} disabled>
                {selected || t('Select Model')}
              </NativeSelectOption>
            )}
            {options.map((model) => (
              <NativeSelectOption key={model} value={model}>
                {model}
              </NativeSelectOption>
            ))}
          </NativeSelect>
          <Button
            type='button'
            size='sm'
            disabled={!canSave}
            onClick={() => void save()}
          >
            {t('Save')}
          </Button>
        </div>
      )}
      {!data && !read.isError && !forbidden && (
        <p role='status' className='text-muted-foreground text-sm'>
          {t('Loading settings...')}
        </p>
      )}
      {data && !options.length && (
        <p role='status' className='text-muted-foreground text-sm'>
          {t('No models available')}
        </p>
      )}
      {state === 'saved' && (
        <p role='status' className='text-sm'>
          {t('Saved')}
        </p>
      )}
      {state === 'failed' && (
        <p role='alert' className='text-destructive text-sm'>
          {t('Save failed')}
        </p>
      )}
      {(read.isError || conflict || forbidden) && (
        <div className='space-y-2'>
          <p role='alert' className='text-destructive text-sm'>
            {t(
              forbidden
                ? 'Access Forbidden'
                : read.isError
                  ? 'Unable to load settings'
                  : 'Settings changed. Reload the current configuration before saving again.'
            )}
          </p>
          <Button
            type='button'
            variant='outline'
            size='sm'
            disabled={read.isFetching}
            onClick={() => void reload()}
          >
            {t('Reload configuration')}
          </Button>
        </div>
      )}
    </div>
  )
}
