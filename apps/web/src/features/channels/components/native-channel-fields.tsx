/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useMutation } from '@tanstack/react-query'
import { useState, type ReactNode } from 'react'
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
}: {
  disabled: boolean
  fetchModels: () => Promise<string[]>
}) {
  const { t } = useTranslation()
  const [search, setSearch] = useState('')
  const catalog = useMutation({ mutationFn: fetchModels })
  const models = catalog.data ?? []
  const filtered = models.filter((model) =>
    model.toLowerCase().includes(search.trim().toLowerCase())
  )

  return (
    <section className='min-w-0 space-y-3'>
      <Button
        type='button'
        variant='outline'
        className='min-h-11'
        disabled={disabled || catalog.isPending}
        onClick={() => catalog.mutate()}
      >
        {catalog.isPending ? t('Loading') : t('Fetch Models')}
      </Button>
      {catalog.isError && (
        <p role='alert' className='text-destructive text-sm'>
          {getServerErrorMessage(catalog.error) ?? t('Failed to load data')}
        </p>
      )}
      {catalog.isSuccess && (
        <>
          <p role='status' className='text-muted-foreground text-sm'>
            {t('Total')}: {models.length}
          </p>
          {models.length > 0 && (
            <Input
              aria-label={t('Search models')}
              placeholder={t('Search models')}
              value={search}
              onChange={(event) => setSearch(event.target.value)}
            />
          )}
          {filtered.length > 0 ? (
            <ul className='bg-muted/40 max-h-64 overflow-y-auto rounded-md p-3'>
              {filtered.map((model) => (
                <li key={model} className='py-1 font-mono text-sm break-all'>
                  {model}
                </li>
              ))}
            </ul>
          ) : (
            <p className='text-muted-foreground text-sm'>
              {t('No models found')}
            </p>
          )}
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
  onChange,
}: {
  document: string
  configuration: NativeConfiguration
  disabled: boolean
  credential?: ReactNode
  fetchModels?: () => Promise<string[]>
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
      {fetchModels && (
        <ChannelModelCatalog disabled={disabled} fetchModels={fetchModels} />
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
