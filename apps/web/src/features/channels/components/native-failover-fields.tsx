/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useId } from 'react'
import { useTranslation } from 'react-i18next'

import { Checkbox } from '@/components/ui/checkbox'
import { Input } from '@/components/ui/input'

import type { NativeModelDraft } from './native-model-draft'

export function NativeFailoverFields({
  value,
  disabled,
  onChange,
}: {
  value: NativeModelDraft['route']['failover']
  disabled: boolean
  onChange: (value: NativeModelDraft['route']['failover']) => void
}) {
  const { t } = useTranslation()
  const id = useId()
  const causes = [
    ['connect', t('Connection failure')],
    ['timeout', t('Timeout')],
    ['rate_limited', t('Rate limit (429)')],
    ['server_error', t('Server error (5xx)')],
  ] as const
  return (
    <section className='min-w-0 space-y-5' aria-label={t('Failover policy')}>
      <h4 className='font-medium'>{t('Failover policy')}</h4>
      <div className='max-w-xl space-y-2'>
        <label htmlFor={`${id}-attempts`} className='text-sm'>
          {t('Maximum attempts')}
        </label>
        <Input
          id={`${id}-attempts`}
          type='number'
          inputMode='numeric'
          min={1}
          max={8}
          step={1}
          required
          className='min-h-11 tabular-nums'
          value={value.max_attempts}
          disabled={disabled}
          aria-describedby={`${id}-attempts-help`}
          onChange={(event) => {
            const raw = event.target.value
            const number = Number(raw)
            onChange({
              ...value,
              max_attempts:
                raw !== '' && Number.isSafeInteger(number) ? number : raw,
            })
          }}
        />
        <p id={`${id}-attempts-help`} className='text-muted-foreground text-sm'>
          {t(
            'Includes the first attempt. Set 1 to disable retries. Only eligible routing targets are tried.'
          )}
        </p>
      </div>
      <fieldset
        className='min-w-0 space-y-3'
        aria-describedby={`${id}-retry-help`}
      >
        <legend className='mb-2 text-sm font-medium'>{t('Retry on')}</legend>
        <div className='grid min-w-0 gap-x-6 @min-[40rem]/routing:grid-cols-2'>
          {causes.map(([cause, label]) => (
            <label
              key={cause}
              className='flex min-h-11 items-center gap-3 text-sm'
            >
              <Checkbox
                checked={value.retry_on.includes(cause)}
                disabled={disabled}
                onCheckedChange={(checked) =>
                  onChange({
                    ...value,
                    retry_on:
                      checked === true
                        ? [
                            ...value.retry_on.filter((item) => item !== cause),
                            cause,
                          ]
                        : value.retry_on.filter((item) => item !== cause),
                  })
                }
              />
              {label}
            </label>
          ))}
        </div>
        <p id={`${id}-retry-help`} className='text-muted-foreground text-sm'>
          {t(
            'Only selected failures can try another target. Retries always stop after output begins.'
          )}
        </p>
        {Number(value.max_attempts) > 1 && value.retry_on.length === 0 && (
          <p className='text-muted-foreground text-sm' role='status'>
            {t('Select at least one failure condition to enable retries.')}
          </p>
        )}
      </fieldset>
      <div className='space-y-2'>
        <label className='flex min-h-11 items-center gap-3 text-sm'>
          <Checkbox
            checked={value.allow_replay_after_send}
            disabled={disabled}
            aria-describedby={`${id}-replay-help`}
            onCheckedChange={(checked) =>
              onChange({ ...value, allow_replay_after_send: checked === true })
            }
          />
          {t('Allow replay after send')}
        </label>
        <p
          id={`${id}-replay-help`}
          className='text-muted-foreground max-w-3xl text-sm'
        >
          {t(
            'Replaying a request that may have reached the provider can cause duplicate work and charges. It also requires explicit request permission, which current public API requests do not grant.'
          )}
        </p>
      </div>
    </section>
  )
}
