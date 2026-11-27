/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import {
  microsToUsd,
  usdToMicros,
} from '@/features/channels/components/native-model-draft'
import type { NativeConfiguration } from '@/features/channels/native-api'

import {
  configuredRouterSelectors,
  priceSelectorKey,
  type PriceSelector,
} from './native-price-coverage'
import type { NativePriceRule } from './native-price-rules'

export function NativeRouterPrices({
  routing,
  rules,
  disabled,
  onApply,
  onRemove,
}: {
  routing: NativeConfiguration['routing']
  rules: NativePriceRule[]
  disabled: boolean
  onApply: (
    selector: PriceSelector,
    multiplier: number,
    stored?: NativePriceRule
  ) => void
  onRemove: (id: number) => void
}) {
  const { t } = useTranslation()
  const [editing, setEditing] = useState<{
    key: string
    multiplier: string
  } | null>(null)
  const eligible = new Map(
    configuredRouterSelectors(routing).map((selector) => [
      priceSelectorKey(selector),
      selector,
    ])
  )
  const prices = new Map(rules.map((rule) => [priceSelectorKey(rule), rule]))
  // Retain existing upstream-cost records when a route becomes unavailable.
  const selectors = new Map(eligible)
  for (const rule of rules) {
    if (rule.billing_mode === 'upstream_cost') {
      selectors.set(priceSelectorKey(rule), rule)
    }
  }
  const rows = [...selectors].filter(
    ([key]) =>
      !prices.has(key) || prices.get(key)?.billing_mode === 'upstream_cost'
  )
  const multiplier = editing ? usdToMicros(editing.multiplier) : null
  const valid =
    multiplier !== null && multiplier > 0 && multiplier <= 1_000_000_000

  return (
    <div className='space-y-4'>
      <p className='text-muted-foreground text-sm'>
        {t(
          'Router rules use actual upstream USD cost. External text calls remain blocked until the supplier guarantees the request budget.'
        )}
      </p>
      {rows.map(([key, selector]) => {
        const stored = prices.get(key)
        return (
          <div
            className='flex flex-wrap items-center justify-between gap-4 border-b py-3'
            key={key}
          >
            <div>
              <p className='font-mono break-all'>{selector.model}</p>
              <p className='text-muted-foreground text-sm'>
                {selector.group} · {selector.speed}
              </p>
            </div>
            {editing?.key === key && eligible.has(key) ? (
              <form
                className='flex flex-wrap items-end gap-2'
                onSubmit={(event) => {
                  event.preventDefault()
                  if (!valid || disabled) return
                  onApply(selector, multiplier, stored)
                  setEditing(null)
                }}
              >
                <label className='space-y-1 text-sm'>
                  <span>{t('Group cost multiplier')}</span>
                  <Input
                    inputMode='decimal'
                    required
                    value={editing.multiplier}
                    onChange={(event) =>
                      setEditing({ key, multiplier: event.target.value })
                    }
                  />
                </label>
                <Button type='submit' disabled={!valid || disabled}>
                  {t('Apply to draft')}
                </Button>
                <Button
                  type='button'
                  variant='ghost'
                  onClick={() => setEditing(null)}
                >
                  {t('Cancel')}
                </Button>
              </form>
            ) : (
              <div className='flex flex-wrap items-center gap-3 text-sm'>
                <span>
                  {stored
                    ? `${microsToUsd(stored.group_multiplier_ppm)}×`
                    : t('Unset price')}
                </span>
                {eligible.has(key) ? (
                  <Button
                    type='button'
                    variant='outline'
                    disabled={disabled}
                    onClick={() =>
                      setEditing({
                        key,
                        multiplier: microsToUsd(
                          stored?.group_multiplier_ppm ?? 1_000_000
                        ),
                      })
                    }
                  >
                    {stored ? t('Edit') : t('Add price rule')}
                  </Button>
                ) : (
                  <span>{t('Capability unavailable')}</span>
                )}
                {stored && (
                  <Button
                    type='button'
                    variant='ghost'
                    disabled={disabled}
                    onClick={() => onRemove(stored.id)}
                  >
                    {t('Remove')}
                  </Button>
                )}
              </div>
            )}
          </div>
        )
      })}
      {rows.length === 0 && (
        <p role='status' className='text-muted-foreground text-sm'>
          {t('No unpriced router route with a supported budget contract')}
        </p>
      )}
      <a href='/channels' className='text-sm underline'>
        {t('Channels')}
      </a>
    </div>
  )
}
