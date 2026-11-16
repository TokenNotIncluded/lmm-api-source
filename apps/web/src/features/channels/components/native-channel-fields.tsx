/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useId } from 'react'
import { useTranslation } from 'react-i18next'

import { Checkbox } from '@/components/ui/checkbox'
import { Input } from '@/components/ui/input'

import { nativeProtocols } from '../lib/native-routing-form'
import { parseNativeDocument, type NativeConfiguration } from '../native-api'
import { NativeChannelModelsFields } from './native-channel-models-fields'

/** Common edits share the exact draft and CAS version with the advanced editor. */
export function NativeChannelFields({
  document,
  configuration,
  disabled,
  onChange,
}: {
  document: string
  configuration: NativeConfiguration
  disabled: boolean
  onChange: (document: string) => void
}) {
  const { t } = useTranslation()
  const id = useId()
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
  const fields = [
    { key: 'name', label: t('Name'), required: true },
    { key: 'tag', label: t('Tag') },
    { key: 'endpoint', label: t('Base URL'), required: true, type: 'url' },
    {
      key: 'protocol',
      label: t('Protocol'),
      required: true,
      list: `${id}-protocols`,
    },
  ]
  return (
    <div className='min-w-0 space-y-7'>
      <div className='grid min-w-0 gap-4 @min-[40rem]/routing:grid-cols-2'>
        {fields.map((field) => (
          <label key={field.key} className='block min-w-0 space-y-2'>
            <span className='text-sm font-medium'>{field.label}</span>
            <Input
              value={
                typeof draft[field.key] === 'string'
                  ? (draft[field.key] as string)
                  : ''
              }
              disabled={disabled}
              required={field.required}
              type={field.type}
              list={field.list}
              className='min-h-11'
              onChange={(event) => change(field.key, event.target.value)}
            />
          </label>
        ))}
        <datalist id={`${id}-protocols`}>
          {nativeProtocols.map((protocol) => (
            <option key={protocol} value={protocol} />
          ))}
        </datalist>
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
