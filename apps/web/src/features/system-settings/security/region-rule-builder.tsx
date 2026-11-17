/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useId, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'

import { createRegionRoutingRules } from './ip-access-routing-config'

export function RegionRuleBuilder({
  onApply,
}: {
  onApply: (rules: string) => void
}) {
  const { t } = useTranslation()
  const inputId = useId()
  const modeId = useId()
  const unknownId = useId()
  const errorId = useId()
  const [codes, setCodes] = useState('')
  const [mode, setMode] = useState<'allow' | 'block'>('block')
  const [unknown, setUnknown] = useState<'direct' | 'reject'>('reject')
  const [error, setError] = useState('')

  function apply() {
    const rules = createRegionRoutingRules(codes, mode, unknown)
    if (!rules) {
      setError(t('Enter 1–64 two-letter country or region codes.'))
      return
    }
    setError('')
    onApply(rules)
  }

  return (
    <div data-settings-form-span='full' className='space-y-5'>
      <div className='space-y-1'>
        <h3 className='text-sm font-medium'>{t('Region rule builder')}</h3>
        <p className='text-muted-foreground text-sm leading-6'>
          {t(
            'Choose regions, generate a draft, then review and save the rules below.'
          )}
        </p>
      </div>
      <div className='grid min-w-0 gap-5 sm:grid-cols-2'>
        <div className='space-y-2'>
          <Label htmlFor={modeId}>{t('Region access mode')}</Label>
          <NativeSelect
            id={modeId}
            value={mode}
            onChange={(event) =>
              setMode(event.target.value as 'allow' | 'block')
            }
          >
            <NativeSelectOption value='block'>
              {t('Block selected regions')}
            </NativeSelectOption>
            <NativeSelectOption value='allow'>
              {t('Allow only selected regions')}
            </NativeSelectOption>
          </NativeSelect>
        </div>
        <div className='space-y-2'>
          <Label htmlFor={unknownId}>{t('Unknown region')}</Label>
          <NativeSelect
            id={unknownId}
            value={unknown}
            onChange={(event) =>
              setUnknown(event.target.value as 'direct' | 'reject')
            }
          >
            <NativeSelectOption value='reject'>
              {t('Reject access')}
            </NativeSelectOption>
            <NativeSelectOption value='direct'>
              {t('Allow access')}
            </NativeSelectOption>
          </NativeSelect>
        </div>
        <div className='space-y-2 sm:col-span-2'>
          <Label htmlFor={inputId}>{t('Country or region codes')}</Label>
          <Input
            id={inputId}
            value={codes}
            onChange={(event) => {
              setCodes(event.target.value)
              setError('')
            }}
            autoCapitalize='characters'
            autoCorrect='off'
            spellCheck={false}
            placeholder='CN, JP, US'
            aria-invalid={Boolean(error)}
            aria-describedby={error ? errorId : undefined}
          />
          {error && (
            <p id={errorId} role='alert' className='text-destructive text-sm'>
              {error}
            </p>
          )}
        </div>
      </div>
      <div className='flex flex-wrap items-center gap-3'>
        <Button type='button' variant='outline' onClick={apply}>
          {t('Replace draft with region rules')}
        </Button>
        <p className='text-muted-foreground text-xs'>
          {t(
            'This replaces the draft, including any IP exceptions. Nothing is saved yet.'
          )}
        </p>
      </div>
    </div>
  )
}
