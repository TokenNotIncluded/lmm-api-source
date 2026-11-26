/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import {
  microsToUsd,
  usdToMicros,
} from '@/features/channels/components/native-model-draft'

import { priceSelectorKey, type PriceSelector } from './native-price-coverage'
import type { NativePriceRule } from './native-price-rules'

type Row = { selector: PriceSelector; rule?: NativePriceRule }

export function NativePriceCoveragePanel({
  rows,
  tools,
  disabled,
  addRule,
  saveFee,
}: {
  rows: Row[]
  tools?: boolean
  disabled: boolean
  addRule: (selector: PriceSelector) => void
  saveFee: (row: Row, micros: number) => void
}) {
  const { t } = useTranslation()
  const [filter, setFilter] = useState('')
  const [page, setPage] = useState(1)
  const [editing, setEditing] = useState<{ key: string; usd: string } | null>(
    null
  )
  const needle = filter.trim().toLowerCase()
  const filtered = rows.filter(({ selector }) =>
    [selector.model, selector.group, selector.speed].some((value) =>
      value.toLowerCase().includes(needle)
    )
  )
  const pages = Math.max(1, Math.ceil(filtered.length / 20))
  const current = Math.min(page, pages)
  return (
    <div className='space-y-4'>
      <Input
        className='max-w-sm'
        aria-label={t('Search models or groups')}
        placeholder={t('Search models or groups')}
        value={filter}
        onChange={(event) => {
          setFilter(event.target.value)
          setPage(1)
        }}
      />
      {filtered.slice((current - 1) * 20, current * 20).map((row) => {
        const { selector, rule } = row
        const key = priceSelectorKey(selector)
        const active = editing?.key === key
        const amount = active ? usdToMicros(editing.usd) : null
        return (
          <div
            key={key}
            className='flex flex-wrap items-center justify-between gap-3 border-b py-3'
          >
            <div className='min-w-0'>
              <p className='font-medium break-all'>{selector.model}</p>
              <p className='text-muted-foreground text-sm'>
                {selector.group} · {selector.speed}
              </p>
            </div>
            {tools && active ? (
              <form
                className='flex flex-wrap items-end gap-2'
                onSubmit={(event) => {
                  event.preventDefault()
                  if (amount === null || disabled) return
                  saveFee(row, amount)
                  setEditing(null)
                }}
              >
                <label className='space-y-1 text-sm'>
                  <span>{t('Request fee (USD / request)')}</span>
                  <Input
                    inputMode='decimal'
                    value={editing.usd}
                    required
                    disabled={disabled}
                    onChange={(event) =>
                      setEditing({ key, usd: event.target.value })
                    }
                  />
                </label>
                <Button type='submit' disabled={disabled || amount === null}>
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
              <div className='flex items-center gap-3'>
                {tools && (
                  <span className='text-muted-foreground text-sm'>
                    {rule
                      ? `${t('Request fee (USD / request)')}: ${microsToUsd(rule.rates.request)}`
                      : t('Unset price')}
                  </span>
                )}
                <Button
                  type='button'
                  variant='outline'
                  disabled={disabled}
                  onClick={() => {
                    if (tools) {
                      setEditing({
                        key,
                        usd: rule ? microsToUsd(rule.rates.request) : '',
                      })
                    } else addRule(selector)
                  }}
                >
                  {rule ? t('Edit') : t('Add price rule')}
                </Button>
              </div>
            )}
          </div>
        )
      })}
      {filtered.length === 0 && (
        <p role='status' className='text-muted-foreground py-6 text-sm'>
          {filter
            ? t('No matching entries')
            : tools
              ? t('No models available')
              : t('No missing models found.')}
        </p>
      )}
      {pages > 1 && (
        <div className='flex items-center gap-3'>
          <Button
            type='button'
            variant='ghost'
            disabled={current === 1}
            onClick={() => setPage(current - 1)}
          >
            {t('Previous')}
          </Button>
          <span className='text-sm tabular-nums'>
            {current} / {pages}
          </span>
          <Button
            type='button'
            variant='ghost'
            disabled={current === pages}
            onClick={() => setPage(current + 1)}
          >
            {t('Next')}
          </Button>
        </div>
      )}
    </div>
  )
}
