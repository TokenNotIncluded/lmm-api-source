/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useId, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { toIntlLocale } from '@/i18n/languages'

import {
  nativeGlobalTimeoutMs,
  targetTimeoutTooShort,
} from '../lib/native-target-timeout'
import type {
  NativeChannelBatchPatch,
  NativeConfiguration,
} from '../native-api'
import { NativeTimeoutHint } from './native-target-timeout-field'

export function NativeChannelBatchEditor({
  configuration,
  enforceMinimum,
  busy,
  onSave,
  onCancel,
}: {
  configuration: NativeConfiguration
  enforceMinimum: boolean
  busy: boolean
  onSave: (patch: NativeChannelBatchPatch) => Promise<void>
  onCancel: () => void
}) {
  const { t, i18n } = useTranslation()
  const formId = useId()
  const [values, setValues] = useState({
    priority: '',
    weight: '',
    timeout_ms: '',
  })
  const minimumTimeout = enforceMinimum
    ? nativeGlobalTimeoutMs(configuration)
    : null
  const timeoutTooShort =
    values.timeout_ms.trim() !== '' &&
    targetTimeoutTooShort(Number(values.timeout_ms), minimumTimeout)
  const fields = [
    { key: 'priority', label: t('Priority'), min: 0, max: 65_535 },
    { key: 'weight', label: t('Weight'), min: 0, max: 4_294_967_295 },
    {
      key: 'timeout_ms',
      label: t('Timeout (ms)'),
      min: minimumTimeout ?? 1,
      max: 3_600_000,
    },
  ] as const
  const patch: NativeChannelBatchPatch = {}
  let invalid = false
  for (const { key, min, max } of fields) {
    if (!values[key].trim()) continue
    const value = Number(values[key])
    if (!Number.isInteger(value) || value < min || value > max) invalid = true
    else patch[key] = value
  }
  const canSave = !busy && !invalid && Object.keys(patch).length > 0
  const number = new Intl.NumberFormat(toIntlLocale(i18n.language))

  return (
    <form
      className='bg-card space-y-4 rounded-xl p-4 sm:p-5'
      aria-label={t('Edit routing')}
      onSubmit={(event) => {
        event.preventDefault()
        if (canSave) void onSave(patch)
      }}
    >
      <div className='space-y-1'>
        <h3 className='text-sm font-medium'>{t('Edit routing')}</h3>
        <p className='text-muted-foreground max-w-2xl text-sm'>
          {t(
            'Apply to all model routes for the selected channels. Leave fields blank to keep current values.'
          )}
        </p>
      </div>
      <div className='grid min-w-0 gap-4 @min-[40rem]/routing:grid-cols-3'>
        {fields.map(({ key, label, min, max }) => (
          <div key={key} className='min-w-0 space-y-2'>
            <div className='flex flex-wrap items-center justify-between gap-2'>
              <Label htmlFor={`${formId}-${key}`}>{label}</Label>
              <span
                id={`${formId}-${key}-range`}
                className='text-muted-foreground text-xs tabular-nums'
              >
                {number.format(min)}–{number.format(max)}
              </span>
            </div>
            <Input
              id={`${formId}-${key}`}
              name={key}
              className='min-h-11 tabular-nums'
              type='number'
              inputMode='numeric'
              min={min}
              max={max}
              step={1}
              placeholder={t('Keep current value')}
              aria-invalid={
                (key === 'timeout_ms' && timeoutTooShort) || undefined
              }
              aria-describedby={
                key === 'timeout_ms'
                  ? `${formId}-${key}-range ${formId}-timeout-hint`
                  : `${formId}-${key}-range`
              }
              value={values[key]}
              disabled={busy}
              onChange={(event) =>
                setValues({ ...values, [key]: event.target.value })
              }
            />
          </div>
        ))}
      </div>
      <p className='text-muted-foreground text-sm'>
        {t(
          'Lower priority values run first. Weight distributes requests within the same priority.'
        )}
      </p>
      <NativeTimeoutHint
        configuration={configuration}
        id={`${formId}-timeout-hint`}
      />
      {invalid && (
        <p className='text-destructive text-sm' role='alert'>
          {timeoutTooShort
            ? t(
                'Target timeout must be at least {{timeout}} ms for enabled channels.',
                { timeout: minimumTimeout }
              )
            : t('Enter whole numbers within the allowed ranges.')}
        </p>
      )}
      <div className='flex flex-wrap gap-2'>
        <Button type='submit' className='min-h-11' disabled={!canSave}>
          {busy ? t('Saving...') : t('Apply')}
        </Button>
        <Button
          type='button'
          variant='ghost'
          className='min-h-11'
          disabled={busy}
          onClick={onCancel}
        >
          {t('Cancel')}
        </Button>
      </div>
    </form>
  )
}
