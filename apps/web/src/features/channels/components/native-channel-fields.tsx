/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import { Checkbox } from '@/components/ui/checkbox'
import { Input } from '@/components/ui/input'

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

/** Common edits share the exact draft and CAS version with the advanced editor. */
export function NativeChannelFields({
  document,
  configuration,
  disabled,
  credential,
  onChange,
}: {
  document: string
  configuration: NativeConfiguration
  disabled: boolean
  credential?: ReactNode
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
    const next = { ...draft, [key]: value }
    if (
      (key === 'endpoint' || key === 'protocol') &&
      Array.isArray(draft.contexts)
    ) {
      next.contexts = draft.contexts.map((context) => ({
        ...context,
        [key]: value,
      }))
    }
    onChange(JSON.stringify(next, null, 2))
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
        <label className='flex min-h-11 items-center gap-3 text-sm @min-[40rem]/routing:col-span-2'>
          <Checkbox
            checked={draft.enabled === true}
            disabled={disabled}
            onCheckedChange={(checked) => change('enabled', checked === true)}
          />
          {t('Enabled')}
        </label>
      </div>
      <NativeChannelModelsFields
        document={document}
        configuration={configuration}
        disabled={disabled}
        onChange={onChange}
      />
    </div>
  )
}
