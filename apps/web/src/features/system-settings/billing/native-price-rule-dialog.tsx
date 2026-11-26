/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Dialog } from '@/components/dialog'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Textarea } from '@/components/ui/textarea'
import {
  microsToUsd,
  usdToMicros,
} from '@/features/channels/components/native-model-draft'
import {
  parseNativeDocument,
  stringifyNativeDocument,
  type NativeConfiguration,
} from '@/features/channels/native-api'

import {
  configuredPriceSpeeds,
  configuredRouterSelectors,
} from './native-price-coverage'
import {
  decimalPriceFraction,
  fractionText,
  nativePriceRuleSchema,
  priceRateKeys,
  type NativePriceRule,
} from './native-price-rules'
import { expressionPriceRule } from './native-pricing-sources'
import { NativeUpstreamPriceSync } from './native-upstream-price-sync'

type Tier = {
  up_to_input_tokens: number | null
  rates: Record<string, unknown>
}
const tierRateKeys = [
  'input',
  'output',
  'cache_read',
  'cache_write',
  'cache_write_1h',
  'cache_write_12h',
] as const

export function NativePriceRuleDialog({
  initial,
  groups,
  models,
  routing,
  openUpstreamSync = false,
  onClose,
  onSave,
}: {
  initial: NativePriceRule
  groups: string[]
  models: string[]
  routing: NativeConfiguration['routing']
  openUpstreamSync?: boolean
  onClose: () => void
  onSave: (rule: NativePriceRule) => void
}) {
  const { t } = useTranslation()
  const [rule, setRule] = useState(initial)
  const [rates, setRates] = useState(
    () =>
      Object.fromEntries(
        priceRateKeys.map((key) => [key, microsToUsd(initial.rates[key])])
      ) as Record<(typeof priceRateKeys)[number], string>
  )
  const [groupMultiplier, setGroupMultiplier] = useState(
    microsToUsd(initial.group_multiplier_ppm)
  )
  const [speedMultiplier, setSpeedMultiplier] = useState(
    microsToUsd(initial.speed_multiplier_ppm)
  )
  const tariff = initial.tiered as
    | { tiers: Tier[]; [key: string]: unknown }
    | undefined
  const [tiers, setTiers] = useState(
    () =>
      tariff?.tiers.map((tier) => ({
        bound:
          tier.up_to_input_tokens === null
            ? ''
            : String(tier.up_to_input_tokens),
        rates: Object.fromEntries(
          Object.entries(tier.rates).map(([key, value]) => [
            key,
            fractionText(value) ?? '',
          ])
        ),
      })) ?? []
  )
  const [expression, setExpression] = useState<string | null>(null)
  const [building, setBuilding] = useState(false)
  const [advanced, setAdvanced] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const labels = {
    input: t('Input price (USD / 1M tokens)'),
    output: t('Output price (USD / 1M tokens)'),
    cache_read: t('Cache read (USD / 1M tokens)'),
    cache_write: t('Cache write (USD / 1M tokens)'),
    cache_write_1h: t('Cache write, 1 hour (USD / 1M tokens)'),
    cache_write_12h: t('Cache write, 12 hours (USD / 1M tokens)'),
    request: t('Request fee (USD / request)'),
  }
  const speeds = configuredPriceSpeeds(routing, rule.model, rule.group)
  const validSpeed = speeds.includes(rule.speed)
  function checkSelector(candidate: NativePriceRule) {
    if (
      !configuredPriceSpeeds(
        routing,
        candidate.model,
        candidate.group
      ).includes(candidate.speed)
    ) {
      throw new Error(t('Capability unavailable'))
    }
    if (
      candidate.billing_mode === 'upstream_cost' &&
      !configuredRouterSelectors(routing).some(
        (selector) =>
          selector.model === candidate.model &&
          selector.group === candidate.group
      )
    ) {
      throw new Error(t('Capability unavailable'))
    }
    return candidate
  }
  async function buildRule() {
    if (advanced !== null) {
      return checkSelector(
        nativePriceRuleSchema.parse(parseNativeDocument(advanced))
      )
    }
    const group = usdToMicros(groupMultiplier)
    const speed = usdToMicros(speedMultiplier)
    if (
      group === null ||
      group > 1_000_000_000 ||
      speed === null ||
      speed === 0 ||
      speed > 1_000_000_000
    ) {
      throw new Error(
        t(
          'Use multipliers from 0 to 1000 with at most six decimal places; the speed multiplier must be above zero.'
        )
      )
    }
    const next: NativePriceRule = {
      ...rule,
      group_multiplier_ppm: group,
      speed_multiplier_ppm: speed,
    }
    if (expression !== null) {
      const price = await expressionPriceRule(
        {
          ...next,
          rates: { ...next.rates, request: usdToMicros(rates.request) ?? -1 },
        },
        expression
      )
      if (!price) {
        throw new Error(
          t(
            'Use a valid linear token expression on a rule without a request fee.'
          )
        )
      }
      return checkSelector(nativePriceRuleSchema.parse(price))
    }
    if (!rule.billing_mode || rule.billing_mode === 'tokens') {
      for (const key of priceRateKeys) {
        const value = usdToMicros(rates[key])
        if (value === null) {
          throw new Error(
            t('Enter a non-negative price with at most six decimal places.')
          )
        }
        next.rates = { ...next.rates, [key]: value }
      }
    } else if (rule.billing_mode === 'tiered_decimal' && tariff) {
      next.tiered = {
        ...tariff,
        tiers: tiers.map((tier, index) => ({
          ...tariff.tiers[index],
          up_to_input_tokens:
            index === tiers.length - 1 ? null : Number(tier.bound),
          rates: Object.fromEntries(
            Object.entries(tier.rates).map(([key, text]) => {
              const fraction = decimalPriceFraction(text)
              if (!fraction) {
                throw new Error(
                  t('Enter an exact non-negative decimal price or fraction.')
                )
              }
              return [key, fraction]
            })
          ),
        })),
      }
      if (
        tiers
          .slice(0, -1)
          .some(
            (tier, index) =>
              !Number.isSafeInteger(Number(tier.bound)) ||
              Number(tier.bound) <=
                (index === 0 ? 0 : Number(tiers[index - 1].bound))
          )
      ) {
        throw new Error(
          t('Tier limits must be increasing positive whole numbers.')
        )
      }
    }
    return checkSelector(nativePriceRuleSchema.parse(next))
  }
  async function attempt(action: (value: NativePriceRule) => void) {
    if (building) return
    setBuilding(true)
    try {
      action(await buildRule())
      setError(null)
    } catch (cause) {
      setError(
        cause instanceof Error ? cause.message : t('Invalid configuration')
      )
    } finally {
      setBuilding(false)
    }
  }
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
      title={t('Edit price rule')}
      description={t(
        'Changes stay in your draft until you preview and publish them.'
      )}
      contentClassName='bg-background sm:max-w-3xl'
      footer={
        <>
          <Button type='button' variant='ghost' onClick={onClose}>
            {t('Cancel')}
          </Button>
          <Button
            type='submit'
            form='native-price-rule-form'
            disabled={building}
          >
            {t('Apply to draft')}
          </Button>
        </>
      }
    >
      <form
        id='native-price-rule-form'
        className='min-w-0 space-y-6'
        onSubmit={(event) => {
          event.preventDefault()
          void attempt(onSave)
        }}
      >
        {advanced === null ? (
          <>
            <div className='grid min-w-0 gap-5 sm:grid-cols-2'>
              <label className='min-w-0 space-y-2 text-sm sm:col-span-2'>
                <span>{t('Model name')}</span>
                <Input
                  className='font-mono'
                  list='native-price-models'
                  required
                  maxLength={200}
                  value={rule.model}
                  onChange={(event) =>
                    setRule({ ...rule, model: event.target.value })
                  }
                />
                <datalist id='native-price-models'>
                  {models.map((model) => (
                    <option value={model} key={model} />
                  ))}
                </datalist>
              </label>
              <label className='space-y-2 text-sm'>
                <span>{t('Group')}</span>
                <NativeSelect
                  className='w-full'
                  value={rule.group}
                  onChange={(event) =>
                    setRule({ ...rule, group: event.target.value })
                  }
                >
                  {[...new Set([...groups, rule.group])].map((group) => (
                    <NativeSelectOption key={group} value={group}>
                      {group}
                    </NativeSelectOption>
                  ))}
                </NativeSelect>
              </label>
              <label className='space-y-2 text-sm'>
                <span>{t('Speed')}</span>
                <NativeSelect
                  className='w-full'
                  value={validSpeed ? rule.speed : ''}
                  disabled={
                    rule.billing_mode === 'upstream_cost' || !speeds.length
                  }
                  onChange={(event) =>
                    setRule({
                      ...rule,
                      speed: event.target.value as NativePriceRule['speed'],
                    })
                  }
                >
                  {!validSpeed && (
                    <NativeSelectOption value=''>
                      {t('Capability unavailable')}
                    </NativeSelectOption>
                  )}
                  {speeds.map((speed) => (
                    <NativeSelectOption key={speed} value={speed}>
                      {speed}
                    </NativeSelectOption>
                  ))}
                </NativeSelect>
              </label>
            </div>
            {(!rule.billing_mode || rule.billing_mode === 'tokens') && (
              <div className='space-y-3'>
                <Button
                  type='button'
                  variant='outline'
                  onClick={() => setExpression(expression === null ? '' : null)}
                >
                  {expression === null
                    ? t('Expression pricing')
                    : t('Use token prices')}
                </Button>
                {expression === null && (
                  <NativeUpstreamPriceSync
                    key={rule.model}
                    model={rule.model}
                    defaultOpen={openUpstreamSync}
                    onApply={(incoming) =>
                      setRates((current) => ({
                        ...current,
                        ...Object.fromEntries(
                          Object.entries(incoming).map(([field, value]) => [
                            field,
                            microsToUsd(value),
                          ])
                        ),
                      }))
                    }
                  />
                )}
              </div>
            )}
            {expression !== null ? (
              <label className='block space-y-2 text-sm'>
                <span>{t('Expression')}</span>
                <Textarea
                  className='min-h-24 font-mono'
                  value={expression}
                  onChange={(event) => setExpression(event.target.value)}
                  placeholder={
                    'tier("standard", p * 1.25 + c * 10 + cr * 0.125)'
                  }
                  maxLength={1024}
                  required
                />
                <span className='text-muted-foreground text-xs'>
                  {t(
                    'Enter one tier with USD per million token coefficients for p and c. You can also include cr and cc.'
                  )}
                </span>
              </label>
            ) : (
              (!rule.billing_mode || rule.billing_mode === 'tokens') && (
                <div className='grid min-w-0 gap-5 sm:grid-cols-2'>
                  {priceRateKeys.map((key) => (
                    <label key={key} className='space-y-2 text-sm'>
                      <span>{labels[key]}</span>
                      <Input
                        className='tabular-nums'
                        inputMode='decimal'
                        required
                        value={rates[key]}
                        onChange={(event) =>
                          setRates({ ...rates, [key]: event.target.value })
                        }
                      />
                    </label>
                  ))}
                </div>
              )
            )}
            {rule.billing_mode === 'upstream_cost' && (
              <p className='text-muted-foreground text-sm leading-6'>
                {t(
                  'Router rules charge the upstream cost multiplied by the group multiplier. Token prices do not apply.'
                )}
              </p>
            )}
            {rule.billing_mode === 'tiered_decimal' && (
              <section className='space-y-6' aria-label={t('Tiered pricing')}>
                <p className='text-muted-foreground text-sm leading-6'>
                  {t(
                    'Each tier applies to the full request based on its total input tokens. Prices accept exact decimals or fractions.'
                  )}
                </p>
                {tiers.map((tier, index) => (
                  <fieldset key={index} className='space-y-4 border-t pt-4'>
                    <legend className='px-1 text-sm font-medium'>
                      {t('Tier {{number}}', { number: index + 1 })}
                    </legend>
                    {index < tiers.length - 1 ? (
                      <label className='block max-w-xs space-y-2 text-sm'>
                        <span>{t('Maximum input tokens')}</span>
                        <Input
                          inputMode='numeric'
                          required
                          value={tier.bound}
                          onChange={(event) =>
                            setTiers(
                              tiers.map((item, position) =>
                                position === index
                                  ? { ...item, bound: event.target.value }
                                  : item
                              )
                            )
                          }
                        />
                      </label>
                    ) : (
                      <p className='text-muted-foreground text-sm'>
                        {t('No upper limit')}
                      </p>
                    )}
                    <div className='grid min-w-0 gap-5 sm:grid-cols-2'>
                      {tierRateKeys
                        .filter((key) => key in tier.rates)
                        .map((key) => (
                          <label key={key} className='space-y-2 text-sm'>
                            <span>{labels[key]}</span>
                            <Input
                              className='tabular-nums'
                              inputMode='decimal'
                              required
                              value={tier.rates[key]}
                              onChange={(event) =>
                                setTiers(
                                  tiers.map((item, position) =>
                                    position === index
                                      ? {
                                          ...item,
                                          rates: {
                                            ...item.rates,
                                            [key]: event.target.value,
                                          },
                                        }
                                      : item
                                  )
                                )
                              }
                            />
                          </label>
                        ))}
                    </div>
                  </fieldset>
                ))}
              </section>
            )}
            <div className='grid gap-5 border-t pt-5 sm:grid-cols-2'>
              <label className='space-y-2 text-sm'>
                <span>{t('Group multiplier')}</span>
                <Input
                  inputMode='decimal'
                  required
                  className='tabular-nums'
                  value={groupMultiplier}
                  onChange={(event) => setGroupMultiplier(event.target.value)}
                />
              </label>
              <label className='space-y-2 text-sm'>
                <span>{t('Speed multiplier')}</span>
                <Input
                  inputMode='decimal'
                  required
                  className='tabular-nums'
                  value={speedMultiplier}
                  disabled={rule.billing_mode === 'upstream_cost'}
                  onChange={(event) => setSpeedMultiplier(event.target.value)}
                />
              </label>
            </div>
            <Button
              type='button'
              variant='ghost'
              onClick={() =>
                void attempt((value) =>
                  setAdvanced(stringifyNativeDocument(value, 2))
                )
              }
            >
              {t('Advanced Settings')}
            </Button>
          </>
        ) : (
          <label className='block space-y-2 text-sm'>
            <span>{t('Configuration (JSON)')}</span>
            <Textarea
              className='min-h-96 font-mono text-xs'
              value={advanced}
              onChange={(event) => setAdvanced(event.target.value)}
            />
          </label>
        )}
        {error && (
          <p role='alert' className='text-destructive text-sm break-words'>
            {error}
          </p>
        )}
      </form>
    </Dialog>
  )
}
