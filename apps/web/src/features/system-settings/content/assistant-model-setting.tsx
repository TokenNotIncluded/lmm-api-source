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
import { ROLE } from '@/lib/roles'

const pluginPath = '/api/assistant/settings/model-route'
const projectionPath = '/api/option/assistant-model'
const version = z.object({
  routing: z.number().int().nonnegative(),
  pricing: z.number().int().nonnegative(),
})
const preferenceEnvelope = z.object({
  success: z.literal(true),
  data: z.object({
    group: z.string(),
    model: z.string(),
    revision: z.number().int().nonnegative(),
  }),
})
const projectionEnvelope = z.object({
  success: z.literal(true),
  data: z.object({
    group: z.string(),
    model: z.string(),
    default_model: z.string(),
    routes: z.record(z.string(), z.array(z.string().min(1))),
    version,
  }),
})
type Draft = {
  group: string
  model: string
  revision: number
  version: z.infer<typeof version>
}

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
  async function load(signal?: AbortSignal) {
    const [plugin, host] = await Promise.all([
      api.get<unknown>(pluginPath, { ...request, signal }),
      api.get<unknown>(projectionPath, { ...request, signal }),
    ])
    const pref = preferenceEnvelope.parse(plugin.data).data
    const effective = projectionEnvelope.parse(host.data).data
    return {
      ...effective,
      revision: pref.revision,
      group: pref.revision ? pref.group : effective.group,
      model: pref.revision
        ? pref.model
        : effective.model || effective.default_model,
      effectiveGroup: effective.group,
      effectiveModel: effective.model || effective.default_model,
    }
  }
  const read = useQuery({
    queryKey,
    retry: false,
    refetchOnWindowFocus: false,
    queryFn: ({ signal }) => load(signal),
  })
  const [draft, setDraft] = useState<Draft>()
  const [state, setState] = useState<
    'idle' | 'saving' | 'saved' | 'failed' | 'conflict' | 'forbidden'
  >('idle')
  const forbidden = state === 'forbidden' || denied(read.error)
  const data = forbidden ? undefined : read.data
  const current = data?.model || data?.default_model || ''
  const group = draft?.group ?? data?.group ?? ''
  const groups = Object.keys(data?.routes ?? {})
  const options = data?.routes[group] ?? []
  const selected = draft?.model ?? current
  const conflict =
    state === 'conflict' ||
    Boolean(
      draft &&
      data &&
      (draft.version.routing !== data.version.routing ||
        draft.version.pricing !== data.version.pricing ||
        draft.revision !== data.revision)
    )
  const pending = Boolean(
    data &&
    (data.group !== data.effectiveGroup || data.model !== data.effectiveModel)
  )
  const editDisabled =
    scope.role !== ROLE.SUPER_ADMIN ||
    state === 'saving' ||
    conflict ||
    read.isError
  const canSave = Boolean(
    data &&
    !editDisabled &&
    ((draft && (selected !== current || group !== data.group)) || pending) &&
    options.includes(selected) &&
    !read.isFetching &&
    state !== 'failed'
  )
  async function save() {
    if (!data || !canSave || !isCurrentNativeUsersScope(scope)) return
    setState('saving')
    try {
      if (group !== data.group || selected !== current || !data.revision) {
        const staged = preferenceEnvelope.parse(
          (
            await api.put<unknown>(
              pluginPath,
              {
                group,
                model: selected,
                revision: data.revision,
              },
              request
            )
          ).data
        ).data
        if (
          staged.group !== group ||
          staged.model !== selected ||
          staged.revision <= data.revision
        ) {
          throw new Error('Unexpected plugin receipt')
        }
      }
      if (!isCurrentNativeUsersScope(scope)) return
      projectionEnvelope.parse(
        (
          await api.put<unknown>(
            projectionPath,
            {
              group,
              model: selected,
              expected_version: data.version,
            },
            request
          )
        ).data
      )
      if (!isCurrentNativeUsersScope(scope)) return
      const fresh = await load()
      if (!isCurrentNativeUsersScope(scope)) return
      client.setQueryData(queryKey, fresh)
      setDraft(undefined)
      if (
        fresh.group !== group ||
        fresh.model !== selected ||
        fresh.effectiveGroup !== group ||
        fresh.effectiveModel !== selected
      ) {
        setState('conflict')
        return
      }
      setState('saved')
      void client.invalidateQueries({
        queryKey: ['assistant-status', scope.userId, scope.sessionId],
      })
    } catch (error) {
      if (!isCurrentNativeUsersScope(scope)) return
      if (denied(error)) {
        setState('forbidden')
      } else if (isAxiosError(error) && error.response?.status === 409) {
        setState('conflict')
      } else {
        setState('failed')
      }
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
      {data && (
        <div className='space-y-2'>
          <Label htmlFor='assistant-plugin-group'>{t('Group')}</Label>
          <NativeSelect
            id='assistant-plugin-group'
            value={group}
            disabled={editDisabled || groups.length === 0}
            onChange={(event) => {
              if (editDisabled) return
              const next = event.target.value
              const choices = data.routes[next] ?? []
              const model = choices.includes(selected)
                ? selected
                : (choices[0] ?? '')
              setDraft({
                group: next,
                model,
                version: draft?.version ?? data.version,
                revision: draft?.revision ?? data.revision,
              })
              setState('idle')
            }}
          >
            {!groups.includes(group) && (
              <NativeSelectOption value={group} disabled>
                {group || t('Unknown')}
              </NativeSelectOption>
            )}
            {groups.map((name) => (
              <NativeSelectOption key={name} value={name}>
                {name}
              </NativeSelectOption>
            ))}
          </NativeSelect>
        </div>
      )}
      <Label htmlFor='assistant-host-model'>{t('Assistant model')}</Label>
      {data && (
        <div className='flex flex-wrap items-center gap-2'>
          <NativeSelect
            id='assistant-host-model'
            value={selected}
            disabled={editDisabled || options.length === 0}
            onChange={(event) => {
              if (editDisabled) return
              const model = event.target.value
              setDraft(
                model === current && group === data.group
                  ? undefined
                  : {
                      group,
                      model,
                      revision: draft?.revision ?? data.revision,
                      version: draft?.version ?? data.version,
                    }
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
      {pending && (
        <p role='status' className='text-sm'>
          {t('Pending')}
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
      {(read.isError || conflict || forbidden || state === 'failed') && (
        <div className='space-y-2'>
          {state !== 'failed' && (
            <p role='alert' className='text-destructive text-sm'>
              {t(
                forbidden
                  ? 'Access Forbidden'
                  : read.isError
                    ? 'Unable to load settings'
                    : 'Settings changed. Reload the current configuration before saving again.'
              )}
            </p>
          )}
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
