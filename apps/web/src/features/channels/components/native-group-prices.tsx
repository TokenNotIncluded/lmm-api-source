/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'

import { nativeConfiguredPrice } from '../lib/native-price-display'
import type { NativeChannelDraft } from '../lib/native-routing-form'
import {
  type NativeConfiguration,
  type NativeModelRoute,
  type NativePrices,
  stringifyNativeDocument,
} from '../native-api'
import { microsToUsd } from './native-model-draft'
import { NativePriceField } from './native-model-fields'

export function NativeChannelPriceSection({
  draft,
  configuration,
  disabled,
  onEditPrices,
}: {
  draft: NativeChannelDraft | null
  configuration: NativeConfiguration
  disabled: boolean
  onEditPrices?: (route: NativeModelRoute) => void
}) {
  const { t } = useTranslation()
  const [selected, setSelected] = useState('')
  const ids = new Set(draft?.attachments.map((item) => item.route_id))
  const routes = configuration.routing.routes.filter((route) =>
    ids.has(route.id)
  )
  const route = routes.find((item) => String(item.id) === selected) ?? routes[0]
  if (!route) {
    return (
      <p className='text-muted-foreground text-sm'>{t('No models selected')}</p>
    )
  }
  return (
    <div className='min-w-0 space-y-4'>
      <div className='flex flex-wrap items-end gap-3'>
        <label className='min-w-0 flex-1 space-y-1 text-sm'>
          <span>{t('Models & Groups')}</span>
          <select
            className='bg-muted/65 block min-h-11 w-full rounded-lg px-3'
            value={String(route.id)}
            onChange={(event) => setSelected(event.target.value)}
          >
            {routes.map((item) => (
              <option key={item.id} value={String(item.id)}>
                {item.model} · {item.group}
              </option>
            ))}
          </select>
        </label>
        {onEditPrices && (
          <Button
            type='button'
            variant='outline'
            className='min-h-11'
            disabled={disabled}
            onClick={() => onEditPrices(route)}
          >
            {t('Edit group prices')}
          </Button>
        )}
      </div>
      <NativeGroupPrices route={route} prices={configuration.prices} />
    </div>
  )
}

type Rule = NativePrices['rules'][number]
type Route = Pick<NativeModelRoute, 'model' | 'group' | 'speeds'>

function TieredGroupPrices({ rule }: { rule: Rule }) {
  const { t } = useTranslation()
  const tariff = rule.tiered as
    | {
        calendar?: unknown
        tiers?: Array<{ up_to_input_tokens?: number | null }>
      }
    | undefined
  const base = {
    ...rule,
    group_multiplier_ppm: 1_000_000,
    speed_multiplier_ppm: 1_000_000,
  }
  return (
    <div className='min-w-0 space-y-2'>
      <h5 className='text-sm font-medium'>{t('Tiered pricing')}</h5>
      {tariff?.calendar ? (
        <details>
          <summary className='cursor-pointer text-sm'>
            {t('Configuration')}
          </summary>
          <pre className='overflow-auto text-xs'>
            {stringifyNativeDocument(tariff, 2)}
          </pre>
        </details>
      ) : (
        tariff?.tiers?.map((tier, index) => (
          <div key={index} className='space-y-2'>
            <div className='text-muted-foreground text-xs'>
              {t('Tier')} {index + 1} · {t('Input tokens')} ≤{' '}
              {tier.up_to_input_tokens ?? '∞'}
            </div>
            <dl className='grid gap-2 text-sm sm:grid-cols-2'>
              {(['input', 'output'] as const).map((key) => (
                <div key={key}>
                  <dt>
                    {key === 'input'
                      ? t('Input price (USD / 1M tokens)')
                      : t('Output price (USD / 1M tokens)')}
                  </dt>
                  <dd>
                    {t('Base price (1×)')}{' '}
                    {nativeConfiguredPrice(base, key, index) ?? '—'}
                  </dd>
                  <dd>
                    {key === 'input'
                      ? t('Effective input price (USD / 1M tokens)')
                      : t('Effective output price (USD / 1M tokens)')}{' '}
                    {nativeConfiguredPrice(rule, key, index) ?? '—'}
                  </dd>
                </div>
              ))}
            </dl>
          </div>
        ))
      )}
    </div>
  )
}

export function NativeGroupPrices({
  route,
  prices,
  onChange,
  disabled = false,
}: {
  route: Route
  prices: NativePrices
  onChange?: (prices: NativePrices) => void
  disabled?: boolean
}) {
  const { t } = useTranslation()
  const scoped = prices.rules.filter(
    (rule) => rule.model === route.model && rule.group === route.group
  )
  const speeds = [
    ...new Set([...route.speeds, ...scoped.map((rule) => String(rule.speed))]),
  ]
  const dimensions = {
    input: t('Input price (USD / 1M tokens)'),
    output: t('Output price (USD / 1M tokens)'),
    cache_read: t('Cache read (USD / 1M tokens)'),
    cache_write: t('Cache write (USD / 1M tokens)'),
    request: t('Request fee (USD / request)'),
  }
  function replace(previous: Rule | undefined, next: Rule) {
    onChange?.({
      ...prices,
      rules: previous
        ? prices.rules.map((rule) => (rule === previous ? next : rule))
        : [...prices.rules, next],
    })
  }
  return (
    <section className='min-w-0 space-y-4' aria-label={t('Group price rules')}>
      <p className='text-muted-foreground text-sm'>
        {t('Channels using this model and group share these prices.')}
      </p>
      {speeds.map((speed) => {
        const rule = scoped.find((item) => item.speed === speed)
        const defaultRule =
          route.group !== 'default' &&
          prices.rules.find(
            (item) =>
              item.model === route.model &&
              item.group === 'default' &&
              item.speed === speed
          )
        const rates = rule?.rates as Record<string, number | string> | undefined
        const tokenRates =
          rule &&
          (!rule.billing_mode || rule.billing_mode === 'tokens') &&
          rates
        const nextId =
          rule?.id ??
          prices.rules.reduce(
            (max, item) => Math.max(max, Number(item.id)),
            0
          ) + 1
        return (
          <fieldset
            key={speed}
            className='min-w-0 space-y-3 border-t pt-3'
            disabled={disabled}
          >
            <legend className='pe-2 text-sm font-medium'>
              {route.model} · {route.group} · {speed}
            </legend>
            {rule ? (
              <>
                {rule.billing_mode === 'tiered_decimal' ||
                rule.billing_mode === 'upstream_cost' ? (
                  rule.billing_mode === 'tiered_decimal' ? (
                    <TieredGroupPrices rule={rule} />
                  ) : (
                    <p className='text-sm'>{t('Upstream cost')}</p>
                  )
                ) : (
                  <dl className='grid gap-3 text-sm sm:grid-cols-2'>
                    {(['input', 'output'] as const).map((key) => (
                      <div key={key}>
                        <dt className='text-muted-foreground'>
                          {key === 'input'
                            ? t('Effective input price (USD / 1M tokens)')
                            : t('Effective output price (USD / 1M tokens)')}
                        </dt>
                        <dd className='font-mono'>
                          {nativeConfiguredPrice(rule, key) ?? '—'}
                        </dd>
                      </div>
                    ))}
                  </dl>
                )}
                {!onChange && tokenRates && (
                  <div className='space-y-2'>
                    <h5 className='text-sm font-medium'>
                      {t('Base price (1×)')}
                    </h5>
                    <dl className='grid gap-2 text-sm sm:grid-cols-2'>
                      {Object.entries(dimensions).map(([key, label]) => (
                        <div key={key}>
                          <dt className='text-muted-foreground'>{label}</dt>
                          <dd>
                            {typeof rates[key] === 'number'
                              ? microsToUsd(rates[key])
                              : '—'}
                          </dd>
                        </div>
                      ))}
                    </dl>
                  </div>
                )}
                {!onChange && (
                  <dl className='flex flex-wrap gap-x-5 gap-y-2 text-sm'>
                    <div>
                      <dt className='text-muted-foreground'>
                        {t('Group multiplier')}
                      </dt>
                      <dd>
                        ×{' '}
                        {typeof rule.group_multiplier_ppm === 'number'
                          ? microsToUsd(rule.group_multiplier_ppm)
                          : '—'}
                      </dd>
                    </div>
                    {tokenRates &&
                      (['cache_read', 'cache_write', 'request'] as const).map(
                        (key) => (
                          <div key={key}>
                            <dt className='text-muted-foreground'>
                              {dimensions[key]}
                            </dt>
                            <dd>{nativeConfiguredPrice(rule, key) ?? '—'}</dd>
                          </div>
                        )
                      )}
                  </dl>
                )}
                {onChange && (
                  <div className='grid min-w-0 gap-4 sm:grid-cols-2'>
                    {typeof rule.group_multiplier_ppm === 'number' ||
                    typeof rule.group_multiplier_ppm === 'string' ? (
                      <NativePriceField
                        label={t('Group multiplier')}
                        multiplier
                        value={rule.group_multiplier_ppm}
                        disabled={disabled}
                        onChange={(value) =>
                          replace(rule, {
                            ...rule,
                            group_multiplier_ppm: value,
                          })
                        }
                      />
                    ) : null}
                    {tokenRates &&
                      Object.entries(dimensions).map(([key, label]) => (
                        <NativePriceField
                          key={key}
                          label={label}
                          value={rates[key]}
                          disabled={disabled}
                          onChange={(value) =>
                            replace(rule, {
                              ...rule,
                              rates: { ...rates, [key]: value },
                            })
                          }
                        />
                      ))}
                  </div>
                )}
              </>
            ) : (
              <p className='text-muted-foreground text-sm'>
                {t('No published price for this speed.')}
              </p>
            )}
            {onChange && defaultRule && Number.isSafeInteger(nextId) && (
              <Button
                type='button'
                variant='outline'
                onClick={() =>
                  replace(rule, {
                    ...defaultRule,
                    id: nextId,
                    group: route.group,
                  })
                }
              >
                {t('Copy prices from the default group')}
              </Button>
            )}
            {onChange &&
              !rule &&
              !defaultRule &&
              Number.isSafeInteger(nextId) && (
                <Button
                  type='button'
                  variant='outline'
                  onClick={() =>
                    replace(undefined, {
                      id: nextId,
                      model: route.model,
                      group: route.group,
                      speed,
                      rates: {
                        input: '',
                        output: '',
                        cache_read: 0,
                        cache_write: 0,
                        request: 0,
                      },
                      group_multiplier_ppm: 1000000,
                      speed_multiplier_ppm: 1000000,
                    })
                  }
                >
                  {t('Add')}
                </Button>
              )}
          </fieldset>
        )
      })}
    </section>
  )
}
