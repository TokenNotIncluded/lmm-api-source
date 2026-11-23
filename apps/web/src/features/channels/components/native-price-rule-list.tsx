/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useTranslation } from 'react-i18next'

import {
  nativePriceRuleSchema,
  priceRateText,
  type PriceRateKey,
} from '@/features/system-settings/billing/native-price-rules'

import type { NativePrices } from '../native-api'
import { microsToUsd } from './native-model-draft'

/** Read the published document with the same exact formatter as the price editor. */
export function NativePriceRuleList({ prices }: { prices: NativePrices }) {
  const { t } = useTranslation()
  const rateLabels: Record<PriceRateKey, string> = {
    input: t('Input price (USD / 1M tokens)'),
    output: t('Output price (USD / 1M tokens)'),
    cache_read: t('Cache read (USD / 1M tokens)'),
    cache_write: t('Cache write (USD / 1M tokens)'),
    request: t('Request fee (USD / request)'),
  }

  return (
    <section
      className='min-w-0 space-y-5'
      aria-label={t('Price rules')}
      data-slot='native-price-rule-list'
    >
      <div className='space-y-2'>
        <div className='flex flex-wrap items-baseline justify-between gap-3'>
          <h2 className='font-medium'>{t('Price rules')}</h2>
          <span className='text-muted-foreground text-xs tabular-nums'>
            {t('Published version')}: {prices.version}
          </span>
        </div>
        <p className='text-muted-foreground max-w-3xl text-sm leading-6'>
          {t(
            'Set model prices in USD, including input, output, cache and request fees. Group and speed multipliers apply to these base prices.'
          )}
        </p>
      </div>
      <div className='divide-border divide-y'>
        {prices.rules.map((raw, index) => {
          const parsed = nativePriceRuleSchema.safeParse(raw)
          const rule = parsed.success ? parsed.data : undefined
          const model = typeof raw.model === 'string' ? raw.model : t('Unknown')
          const mode = !rule
            ? t('Unknown')
            : rule.billing_mode === 'tiered_decimal'
              ? t('Tiered pricing')
              : rule.billing_mode === 'upstream_cost'
                ? t('Upstream cost')
                : t('Token pricing')
          return (
            <article
              key={String(raw.id ?? index)}
              aria-label={model}
              className='grid min-w-0 gap-5 py-6 first:pt-0 @min-[52rem]/routing:grid-cols-[minmax(14rem,1fr)_minmax(0,2fr)]'
            >
              <div className='min-w-0 space-y-2'>
                <h3 className='font-medium [overflow-wrap:anywhere]'>
                  {model}
                </h3>
                <div className='text-muted-foreground flex flex-wrap gap-x-4 gap-y-1 text-xs [overflow-wrap:anywhere]'>
                  <span>
                    {t('Group')}: {rule?.group ?? t('Unknown')}
                  </span>
                  <span>
                    {t('Speed')}: {rule?.speed ?? t('Unknown')}
                  </span>
                </div>
                <p className='text-muted-foreground text-xs'>{mode}</p>
              </div>
              <dl className='grid min-w-0 grid-cols-2 gap-x-5 gap-y-4 @min-[52rem]/routing:grid-cols-3'>
                {(Object.keys(rateLabels) as PriceRateKey[]).map((key) => {
                  const value = rule ? priceRateText(rule, key) : null
                  const notApplicable =
                    rule?.billing_mode === 'upstream_cost' ||
                    (rule?.billing_mode === 'tiered_decimal' &&
                      key === 'request')
                  return (
                    <div key={key} className='min-w-0 space-y-1'>
                      <dt className='text-muted-foreground text-xs leading-5'>
                        {rateLabels[key]}
                      </dt>
                      <dd className='font-medium [overflow-wrap:anywhere] tabular-nums'>
                        {value ?? (notApplicable ? '—' : t('Unknown'))}
                      </dd>
                    </div>
                  )
                })}
                <div className='min-w-0 space-y-1'>
                  <dt className='text-muted-foreground text-xs leading-5'>
                    {t('Group multiplier')} × {t('Speed multiplier')}
                  </dt>
                  <dd className='font-medium [overflow-wrap:anywhere] tabular-nums'>
                    {rule
                      ? `${microsToUsd(rule.group_multiplier_ppm)} × ${microsToUsd(rule.speed_multiplier_ppm)}`
                      : t('Unknown')}
                  </dd>
                </div>
              </dl>
            </article>
          )
        })}
      </div>
      {prices.rules.length > 0 && (
        <p className='text-muted-foreground max-w-3xl text-xs leading-5'>
          {t(
            'Token prices are USD per 1M tokens; request fees are USD per request. Tier prices follow the configured tier order. A dash means that dimension does not apply.'
          )}
        </p>
      )}
    </section>
  )
}
