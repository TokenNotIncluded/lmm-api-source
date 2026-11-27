/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useMutation } from '@tanstack/react-query'
import { useMemo, useState, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Input } from '@/components/ui/input'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { getServerErrorMessage } from '@/lib/server-error-message'

import {
  presetProtocolEndpoint,
  resolveNativeProvider,
} from '../lib/native-provider'
import {
  nativeProtocolLabel,
  nativeProtocols,
  readChannelDraft,
  toggleChannelAttachment,
  type NativeChannelDraft,
} from '../lib/native-routing-form'
import {
  parseNativeDocument,
  stringifyNativeDocument,
  type NativeConfiguration,
} from '../native-api'
import { NativeChannelModelsFields } from './native-channel-models-fields'
import { NativeProviderLogo } from './native-provider-logo'

export function NativeChannelTagField({
  document,
  disabled,
  onChange,
}: {
  document: string
  disabled: boolean
  onChange: (document: string) => void
}) {
  const { t } = useTranslation()
  const draft = readChannelDraft(document)
  if (!draft) return null
  return (
    <label className='block min-w-0 space-y-2'>
      <span className='text-sm font-medium'>{t('Tag')}</span>
      <Input
        className='min-h-11'
        value={draft.tag ?? ''}
        disabled={disabled}
        onChange={(event) =>
          onChange(
            stringifyNativeDocument({ ...draft, tag: event.target.value }, 2)
          )
        }
      />
    </label>
  )
}

function ChannelModelCatalog({
  disabled,
  fetchModels,
  savedModels,
  routes,
  draft,
  onApply,
  onRemove,
  onCreate,
}: {
  disabled: boolean
  fetchModels?: () => Promise<string[]>
  savedModels?: readonly string[]
  routes: NativeConfiguration['routing']['routes']
  draft: NativeChannelDraft | null
  onApply: (routes: NativeConfiguration['routing']['routes']) => void
  onRemove: (attachments: NativeChannelDraft['attachments']) => void
  onCreate?: (model: string) => void
}) {
  const { t } = useTranslation()
  const [search, setSearch] = useState('')
  const [selected, setSelected] = useState<number[]>([])
  const catalog = useMutation({ mutationFn: fetchModels })
  const availableModels = catalog.data ?? savedModels
  const models = new Set(availableModels)
  const routesByModel = useMemo(() => {
    const index = new Map<string, typeof routes>()
    for (const route of routes) {
      const matches = index.get(route.model)
      if (matches) matches.push(route)
      else index.set(route.model, [route])
    }
    return index
  }, [routes])
  const attached = new Set(draft?.attachments.map((item) => item.route_id))
  const missing =
    catalog.isSuccess && models.size > 0
      ? (draft?.attachments.filter(
          (item) => !models.has(item.upstream_model)
        ) ?? [])
      : []
  const removingLastModel =
    draft?.enabled === true && missing.length === draft.attachments.length
  const selection = new Set(selected)
  const selectedRoutes = routes.filter(
    (route) =>
      selection.has(route.id) &&
      !attached.has(route.id) &&
      models.has(route.model)
  )
  const filtered = [...models].filter((model) =>
    model.toLowerCase().includes(search.trim().toLowerCase())
  )

  return (
    <section className='min-w-0 space-y-3'>
      {fetchModels && (
        <Button
          type='button'
          variant='outline'
          className='min-h-11'
          disabled={disabled || catalog.isPending}
          onClick={() => {
            setSelected([])
            catalog.mutate()
          }}
        >
          {catalog.isPending ? t('Loading') : t('Fetch Models')}
        </Button>
      )}
      {catalog.isError && (
        <p role='alert' className='text-destructive text-sm'>
          {getServerErrorMessage(catalog.error) ?? t('Failed to load data')}
        </p>
      )}
      {missing.length > 0 && (
        <div className='border-border space-y-2 rounded-md border p-3'>
          <p className='text-sm font-medium'>
            {t('Models missing from the upstream catalog')}
          </p>
          <ul className='max-h-32 space-y-1 overflow-y-auto font-mono text-xs'>
            {[...new Set(missing.map((item) => item.upstream_model))].map(
              (model) => (
                <li key={model} className='break-all'>
                  {model}
                </li>
              )
            )}
          </ul>
          {removingLastModel && (
            <p className='text-muted-foreground text-xs'>
              {t(
                'Disable the channel before removing its last configured model.'
              )}
            </p>
          )}
          <Button
            type='button'
            variant='outline'
            disabled={disabled || removingLastModel}
            onClick={() => onRemove(missing)}
          >
            {t('Remove from draft')}
          </Button>
        </div>
      )}
      {availableModels && (
        <>
          <p role='status' className='text-muted-foreground text-sm'>
            {t('Total')}: {models.size}
          </p>
          {models.size > 0 && (
            <Input
              aria-label={t('Search models')}
              placeholder={t('Search models')}
              value={search}
              onChange={(event) => setSearch(event.target.value)}
            />
          )}
          {filtered.length > 0 ? (
            <ul className='bg-muted/40 max-h-64 overflow-y-auto rounded-md p-3'>
              {filtered.map((model) => {
                const matches = routesByModel.get(model) ?? []
                return (
                  <li key={model} className='space-y-1 py-2'>
                    <span className='font-mono text-sm break-all'>{model}</span>
                    {matches.map((route) => (
                      <label
                        key={route.id}
                        className='flex min-h-10 items-center gap-2 text-sm'
                      >
                        <Checkbox
                          checked={
                            attached.has(route.id) || selection.has(route.id)
                          }
                          disabled={disabled || attached.has(route.id)}
                          onCheckedChange={(checked) =>
                            setSelected((ids) =>
                              checked
                                ? [...new Set([...ids, route.id])]
                                : ids.filter((id) => id !== route.id)
                            )
                          }
                        />
                        {route.group}
                      </label>
                    ))}
                    {onCreate && matches.length === 0 && (
                      <Button
                        type='button'
                        variant='ghost'
                        disabled={disabled}
                        onClick={() => onCreate(model)}
                      >
                        {t('Add Model')}
                      </Button>
                    )}
                  </li>
                )
              })}
            </ul>
          ) : (
            <p className='text-muted-foreground text-sm'>
              {t('No models found')}
            </p>
          )}
          <Button
            type='button'
            disabled={disabled || selectedRoutes.length === 0}
            onClick={() => {
              onApply(selectedRoutes)
              setSelected([])
            }}
          >
            {t('Apply')}
          </Button>
        </>
      )}
    </section>
  )
}

/** Common edits share the exact draft and CAS version with the advanced editor. */
export function NativeChannelFields({
  document,
  configuration,
  disabled,
  credential,
  fetchModels,
  onCreateModel,
  onChange,
}: {
  document: string
  configuration: NativeConfiguration
  disabled: boolean
  credential?: ReactNode
  fetchModels?: () => Promise<string[]>
  onCreateModel?: (model: string) => void
  onChange: (document: string) => void
}) {
  const { t } = useTranslation()
  let draft: Record<string, unknown>
  try {
    draft = parseNativeDocument(document)
  } catch {
    // Keep invalid advanced input intact so the operator can correct it.
    return null
  }
  function change(key: string, value: string | boolean) {
    const updates: Record<string, string | boolean> = { [key]: value }
    if (key === 'protocol' && typeof value === 'string') {
      const endpoint = presetProtocolEndpoint(
        provider,
        draft.protocol,
        draft.endpoint,
        value
      )
      if (endpoint) updates.endpoint = endpoint
    }
    const next = { ...draft, ...updates }
    if (
      (key === 'endpoint' || key === 'protocol') &&
      Array.isArray(draft.contexts)
    ) {
      next.contexts = draft.contexts.map((context) => ({
        ...context,
        ...updates,
      }))
    }
    onChange(stringifyNativeDocument(next, 2))
  }
  const savedChannel = configuration.routing.upstreams.find(
    (channel) => channel.id === draft.id
  )
  const catalogMatchesDraft =
    savedChannel &&
    savedChannel.endpoint === draft.endpoint &&
    savedChannel.protocol === draft.protocol
  const savedModels =
    savedChannel && configuration.bindings.discoveries[savedChannel.id]?.models
  const presets = configuration.provider_presets ?? []
  const provider = resolveNativeProvider(
    configuration,
    draft.provider,
    draft.protocol,
    draft.endpoint
  )
  function selectProvider(id: string | null) {
    const preset = presets.find((item) => item.id === id)
    if (!preset) return
    const next: Record<string, unknown> = { ...draft, provider: preset.id }
    if (
      !configuration.routing.upstreams.some((item) => item.id === draft.id) &&
      preset.id !== 'custom'
    ) {
      const defaults = { protocol: preset.protocol, endpoint: preset.endpoint }
      Object.assign(next, defaults)
      if (Array.isArray(draft.contexts)) {
        next.contexts = draft.contexts.map((context) => ({
          ...context,
          ...defaults,
        }))
      }
    }
    onChange(stringifyNativeDocument(next, 2))
  }
  return (
    <div className='min-w-0 space-y-4'>
      <div className='grid min-w-0 gap-x-5 gap-y-4 @min-[40rem]/routing:grid-cols-2'>
        <label className='block min-w-0 space-y-2'>
          <span className='text-sm font-medium'>{t('Name')}</span>
          <Input
            className='min-h-11'
            value={typeof draft.name === 'string' ? draft.name : ''}
            required
            disabled={disabled}
            onChange={(event) => change('name', event.target.value)}
          />
        </label>
        <label className='block min-w-0 space-y-2'>
          <span className='text-sm font-medium'>{t('Protocol')}</span>
          <select
            aria-label={t('Protocol')}
            value={typeof draft.protocol === 'string' ? draft.protocol : ''}
            required
            disabled={disabled}
            className='border-input bg-background block min-h-11 w-full rounded-md border px-3 text-sm'
            onChange={(event) => change('protocol', event.target.value)}
          >
            {nativeProtocols.map((protocol) => (
              <option key={protocol} value={protocol}>
                {nativeProtocolLabel(protocol, t)}
              </option>
            ))}
          </select>
        </label>
        <label className='block min-w-0 space-y-2'>
          <span className='text-sm font-medium'>{t('Base URL')}</span>
          <Input
            type='url'
            className='min-h-11'
            value={typeof draft.endpoint === 'string' ? draft.endpoint : ''}
            required
            disabled={disabled}
            onChange={(event) => change('endpoint', event.target.value)}
          />
        </label>
        {credential}
        {presets.length > 0 && (
          <>
            <div className='space-y-2'>
              <span className='text-sm font-medium'>{t('Provider')}</span>
              <Select
                value={provider?.id ?? 'custom'}
                disabled={disabled}
                onValueChange={selectProvider}
              >
                <SelectTrigger
                  aria-label={t('Provider')}
                  className='min-h-11 w-full'
                >
                  <SelectValue>
                    <NativeProviderLogo icon={provider?.icon} />
                    {provider?.id === 'custom' ? t('Custom') : provider?.label}
                  </SelectValue>
                </SelectTrigger>
                <SelectContent alignItemWithTrigger={false}>
                  {presets.map((preset) => (
                    <SelectItem
                      key={preset.id}
                      value={preset.id}
                      label={preset.label}
                    >
                      <span className='flex items-center gap-2'>
                        <NativeProviderLogo icon={preset.icon} />
                        {preset.id === 'custom' ? t('Custom') : preset.label}
                      </span>
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <label className='block min-w-0 space-y-2'>
              <span className='text-sm font-medium'>{t('Internal notes')}</span>
              <Input
                maxLength={4096}
                disabled={disabled}
                value={typeof draft.note === 'string' ? draft.note : ''}
                onChange={(event) => change('note', event.target.value)}
              />
            </label>
          </>
        )}
        <label className='flex min-h-11 items-center gap-3 text-sm @min-[40rem]/routing:col-span-2'>
          <Checkbox
            checked={draft.enabled === true}
            disabled={disabled}
            onCheckedChange={(checked) => change('enabled', checked === true)}
          />
          {t('Enabled')}
        </label>
      </div>
      {catalogMatchesDraft && (fetchModels || savedModels) && (
        <ChannelModelCatalog
          disabled={disabled}
          fetchModels={fetchModels}
          savedModels={savedModels}
          routes={configuration.routing.routes}
          draft={readChannelDraft(document)}
          onCreate={onCreateModel}
          onRemove={(attachments) => {
            const current = readChannelDraft(document)
            if (!current) return
            onChange(
              stringifyNativeDocument(
                attachments.reduce(
                  (next, item) =>
                    toggleChannelAttachment(
                      next,
                      {
                        id: item.route_id,
                        model: item.upstream_model,
                        speeds: item.speeds,
                      },
                      false
                    ),
                  current
                ),
                2
              )
            )
          }}
          onApply={(routes) => {
            const current = readChannelDraft(document)
            if (!current) return
            onChange(
              stringifyNativeDocument(
                routes.reduce(
                  (next, route) =>
                    toggleChannelAttachment(next, route, true, configuration),
                  current
                ),
                2
              )
            )
          }}
        />
      )}
      <NativeChannelModelsFields
        document={document}
        configuration={configuration}
        disabled={disabled}
        onChange={onChange}
      />
    </div>
  )
}
