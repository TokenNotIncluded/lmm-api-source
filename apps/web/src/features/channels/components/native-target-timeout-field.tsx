/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useId } from 'react'
import { useTranslation } from 'react-i18next'

import { Input } from '@/components/ui/input'

import {
  nativeGlobalTimeoutMs,
  targetTimeoutTooShort,
} from '../lib/native-target-timeout'
import type { NativeConfiguration } from '../native-api'

export function NativeTimeoutHint({
  configuration,
  id,
}: {
  configuration: Pick<NativeConfiguration, 'model_limits'>
  id?: string
}) {
  const { t } = useTranslation()
  const timeout = nativeGlobalTimeoutMs(configuration)
  return (
    <p id={id} className='text-muted-foreground text-xs leading-relaxed'>
      {timeout === null
        ? t(
            'This server has not reported its global timeout. Target timeouts are validated by the server when you save.'
          )
        : t(
            'Each request has a {{timeout}} ms global time limit. Enabled channels require at least this target timeout; larger values do not extend execution.',
            { timeout }
          )}
    </p>
  )
}

export function NativeTargetTimeoutField({
  value,
  configuration,
  enabled,
  disabled,
  onChange,
}: {
  value: number | string
  configuration: NativeConfiguration
  enabled: boolean
  disabled: boolean
  onChange: (value: number | string) => void
}) {
  const { t } = useTranslation()
  const id = useId()
  const minimum = enabled ? nativeGlobalTimeoutMs(configuration) : null
  const invalid = targetTimeoutTooShort(value, minimum)
  return (
    <div className='min-w-0 space-y-2 text-sm'>
      <label htmlFor={id} className='block space-y-2'>
        <span>{t('Timeout (ms)')}</span>
        <Input
          id={id}
          type='number'
          min={minimum ?? 1}
          max={3_600_000}
          step={1}
          required
          className='min-h-11 tabular-nums'
          value={value}
          disabled={disabled}
          aria-invalid={invalid || undefined}
          aria-describedby={invalid ? `${id}-hint ${id}-error` : `${id}-hint`}
          onChange={(event) => {
            const raw = event.target.value
            const number = Number(raw)
            onChange(raw !== '' && Number.isSafeInteger(number) ? number : raw)
          }}
        />
      </label>
      <NativeTimeoutHint configuration={configuration} id={`${id}-hint`} />
      {invalid && (
        <p id={`${id}-error`} role='alert' className='text-destructive text-xs'>
          {t(
            'Target timeout must be at least {{timeout}} ms for enabled channels.',
            {
              timeout: minimum,
            }
          )}
        </p>
      )}
    </div>
  )
}
