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
  selectedIds,
  enforceMinimum,
  busy,
  writable = true,
  onSave,
  onCancel,
}: {
  configuration: NativeConfiguration
  selectedIds: number[]
  enforceMinimum: boolean
  busy: boolean
  writable?: boolean
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
  const [page, setPage] = useState(0)
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
  const canSave = writable && !busy && !invalid && Object.keys(patch).length > 0
  const number = new Intl.NumberFormat(toIntlLocale(i18n.language))

  const selected = new Set(selectedIds)
  const changes = invalid
    ? []
    : configuration.routing.routes.flatMap((route) =>
        route.targets
          .filter((target) => selected.has(target.upstream_id))
          .flatMap((target) => {
            const modified = fields.filter(
              ({ key }) =>
                patch[key] !== undefined && patch[key] !== target[key]
            )
            return modified.length ? [{ route, target, modified }] : []
          })
      )
  const visiblePage = Math.min(
    page,
    Math.max(0, Math.ceil(changes.length / 15) - 1)
  )
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
      {changes.length > 0 && (
        <section aria-label={t('Preview changes')} className='space-y-2'>
          <h4 className='text-sm font-medium'>
            {t('Preview changes')} · {configuration.version.routing} →{' '}
            {configuration.version.routing + 1}
          </h4>
          <div className='overflow-x-auto'>
            <table className='w-full text-start text-sm'>
              <thead>
                <tr>
                  {[
                    t('Channel'),
                    t('Model'),
                    t('Group'),
                    t('Target'),
                    t('Change'),
                  ].map((label) => (
                    <th className='p-2 text-start' key={label}>
                      {label}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {changes
                  .slice(15 * visiblePage, 15 * (visiblePage + 1))
                  .map(({ route, target, modified }) => (
                    <tr key={`${route.id}-${target.id}`}>
                      <td className='p-2'>#{target.upstream_id}</td>
                      <td className='p-2'>{route.model}</td>
                      <td className='p-2'>{route.group}</td>
                      <td className='p-2'>#{target.id}</td>
                      <td className='p-2'>
                        {modified.map(({ key, label }) => (
                          <div key={key}>
                            {label} · {target[key]} → {patch[key]}
                          </div>
                        ))}
                      </td>
                    </tr>
                  ))}
              </tbody>
            </table>
          </div>
          {changes.length > 15 && (
            <div className='flex gap-2'>
              <Button
                type='button'
                variant='ghost'
                disabled={!visiblePage}
                onClick={() => setPage(visiblePage - 1)}
              >
                {t('Previous')}
              </Button>
              <Button
                type='button'
                variant='ghost'
                disabled={(visiblePage + 1) * 15 >= changes.length}
                onClick={() => setPage(visiblePage + 1)}
              >
                {t('Next')}
              </Button>
            </div>
          )}
        </section>
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
