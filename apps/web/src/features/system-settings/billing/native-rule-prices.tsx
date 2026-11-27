/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useState, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import { microsToUsd } from '@/features/channels/components/native-model-draft'
import { type NativeConfiguration } from '@/features/channels/native-api'

import { publishedPriceBands } from './native-price-bands'
import { samePriceRule, type NativePriceRule } from './native-price-rules'

const labels = {
  input: 'Input',
  output: 'Output',
  cache_read: 'Cache read',
  cache_write: 'Cache write',
  cache_write_1h: 'Cache write (1h)',
  cache_write_12h: 'Cache write (12h)',
  request: 'Request fee',
} as const

function Amounts({
  rule,
  published,
  protocols,
}: {
  rule: NativePriceRule
  published: boolean
  protocols: string[]
}) {
  const { t } = useTranslation()
  const { bands, hasCalendar, calendarExpression, revision } =
    publishedPriceBands(rule)
  if (rule.billing_mode === 'upstream_cost')
    return (
      <p>
        {t('Upstream receipt cost')} × {microsToUsd(rule.group_multiplier_ppm)}{' '}
        · {t('Amount pending receipt')}
      </p>
    )
  if (!bands.length) return <p>{t('Rate unavailable')}</p>
  const [first] = bands
  const primary = first.prices.filter(
    (price) => price.meter === 'input' || price.meter === 'output'
  )
  return (
    <details>
      <summary className='cursor-pointer tabular-nums'>
        {primary.map((price) => (
          <span className='me-3 inline-block' key={price.meter}>
            {t(labels[price.meter])} {price.base ?? t('Unknown')} →{' '}
            {price.final ?? t('Unknown')}
          </span>
        ))}
        <span className='text-muted-foreground text-xs'>
          {bands.length > 1
            ? t('{{count}} price bands', { count: bands.length })
            : t('All rates')}
        </span>
      </summary>
      <div className='mt-2 space-y-3'>
        {bands.map((band, index) => (
          <div key={index}>
            <div className='font-medium'>
              {hasCalendar && (
                <span>
                  {t(
                    band.branch === 'calendar'
                      ? 'Calendar match'
                      : 'Calendar default'
                  )}{' '}
                  ·{' '}
                </span>
              )}
              {t('Input tokens')}{' '}
              {band.lower !== null ? '> ' + band.lower : '≥ 0'}
              {band.upper !== null && ' · ≤ ' + band.upper}
            </div>
            <table className='mt-1 w-full text-start text-xs tabular-nums'>
              <thead>
                <tr>
                  <th className='text-start'>{t('Meter')}</th>
                  <th className='text-end'>{t('Base rate')}</th>
                  <th className='text-end'>
                    {t(published ? 'Published unit rate' : 'Draft unit rate')}
                  </th>
                </tr>
              </thead>
              <tbody>
                {band.prices.map((price) => (
                  <tr key={price.meter}>
                    <th scope='row' className='text-start font-normal'>
                      {t(
                        price.meter === 'cache_write' && revision === 4
                          ? 'Cache write (30m)'
                          : labels[price.meter]
                      )}
                      {price.meter === 'request' && (
                        <span className='text-muted-foreground'>
                          {' '}
                          · USD/{t('Request')}
                        </span>
                      )}
                    </th>
                    {price.state === 'unused' || price.state === 'target' ? (
                      <td
                        colSpan={2}
                        className='text-muted-foreground text-end'
                      >
                        {t(
                          price.state === 'target'
                            ? 'Depends on target'
                            : 'No separate rate'
                        )}
                      </td>
                    ) : (
                      <>
                        <td className='text-end'>
                          {price.base ?? t('Unknown')}
                        </td>
                        <td className='text-end'>
                          {price.final ?? t('Unknown')}
                        </td>
                      </>
                    )}
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ))}
        {hasCalendar && (
          <p className='text-muted-foreground max-w-lg text-xs break-words whitespace-normal'>
            {calendarExpression ?? t('Calendar condition unavailable')}
          </p>
        )}
        {rule.billing_mode === 'official_flat' && (
          <div className='space-y-1 text-xs'>
            {t(
              'Uniform cache-write template. The selected upstream determines which lifetimes apply.'
            )}
            {protocols.map((protocol) => (
              <div key={protocol}>
                {protocol} ·{' '}
                {t(
                  protocol === 'openai_chat' || protocol === 'openai_responses'
                    ? 'Uniform write rate for 30m and 12h'
                    : protocol === 'anthropic_messages'
                      ? 'Uniform write rate for 5m and 1h'
                      : protocol === 'gemini'
                        ? 'No separate cache write'
                        : 'Check the target billing contract'
                )}
              </div>
            ))}
          </div>
        )}
      </div>
    </details>
  )
}

export function NativeRulePrices({
  rule,
  current,
  configuration,
  cached,
}: {
  rule: NativePriceRule
  current?: NativePriceRule
  configuration: NativeConfiguration
  cached: boolean
}) {
  const { t } = useTranslation()
  const changed = !current || !samePriceRule(current, rule)
  const official = configuration.bindings.official_prices?.models
  const source =
    official && Object.hasOwn(official, rule.model)
      ? official[rule.model]
      : undefined
  const channels = new Map(
    configuration.routing.upstreams
      .filter((channel) => channel.enabled)
      .map((channel) => [channel.id, channel])
  )
  const targets = configuration.routing.groups.some(
    (g) => g.name === rule.group && g.enabled
  )
    ? configuration.routing.routes
        .filter(
          (route) =>
            route.enabled &&
            route.model === rule.model &&
            route.group === rule.group &&
            route.speeds.includes(rule.speed)
        )
        .flatMap((route) =>
          route.targets
            .filter(
              (target) => target.enabled && target.speeds.includes(rule.speed)
            )
            .flatMap((target) => {
              const channel = channels.get(target.upstream_id)
              return channel
                ? [
                    {
                      label:
                        '#' +
                        channel.id +
                        ' · ' +
                        target.upstream_model +
                        ' · ' +
                        channel.protocol,
                      protocol:
                        target.payload === 'legacy_openrouter_chat_usage_v1'
                          ? target.payload
                          : channel.protocol,
                    },
                  ]
                : []
            })
        )
    : []
  const protocols = [...new Set(targets.map((target) => target.protocol))]
  const version = configuration.version
  const verified =
    version &&
    Number.isSafeInteger(version.routing) &&
    Number.isSafeInteger(version.pricing) &&
    version.routing === configuration.routing.version &&
    version?.pricing === configuration.prices.version
  const { digest } = publishedPriceBands(rule)
  return (
    <div className='min-w-56 space-y-1 whitespace-normal'>
      <div className='text-muted-foreground text-xs'>
        {t(changed ? 'Draft unit rate' : 'Published unit rate')} · USD / 1M{' '}
        {t('Tokens')}
      </div>
      <Amounts rule={rule} published={!changed} protocols={protocols} />
      {changed && (
        <details>
          <summary className='cursor-pointer'>
            {t('Published unit rate')}
          </summary>
          {current ? (
            <Amounts rule={current} published protocols={protocols} />
          ) : (
            t('No published rule')
          )}
        </details>
      )}
      <details className='text-muted-foreground text-xs'>
        <summary className='cursor-pointer'>
          {t('Catalog source')} · {source?.source ?? t('Not recorded')}
          {' · '}
          {t(cached ? 'Last loaded version' : 'Published version')}{' '}
          {verified
            ? t('routing {{routing}} / pricing {{pricing}}', {
                routing: version.routing,
                pricing: version.pricing,
              })
            : t('Unknown')}
        </summary>
        <div className='mt-1 space-y-1'>
          {source?.source_unit && (
            <div>
              {t('Source unit')} · {source.source_unit}
            </div>
          )}
          {digest && (
            <div className='font-mono break-all'>SHA-256 · {digest}</div>
          )}
          <div>
            {t('Routing targets')} ·{' '}
            {targets.length
              ? [...new Set(targets.map((target) => target.label))].join(' / ')
              : t('No enabled target')}
          </div>
          <div>
            {t('Group multiplier')} {microsToUsd(rule.group_multiplier_ppm)}× ·{' '}
            {t('Speed multiplier')} {microsToUsd(rule.speed_multiplier_ppm)}×
          </div>
        </div>
      </details>
    </div>
  )
}

export function GroupPriceRows({
  rules,
  render,
}: {
  rules: NativePriceRule[]
  render: (rule: NativePriceRule) => ReactNode
}) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  if (!rules.length) return <span>{t('No published rule')}</span>
  return (
    <details onToggle={(event) => setOpen(event.currentTarget.open)}>
      <summary className='cursor-pointer'>
        {t('Model prices')} · {rules.length}
      </summary>
      {open && (
        <div className='mt-2 space-y-3'>
          {rules.map((rule) => (
            <div key={rule.id}>
              <div className='font-mono text-xs'>
                {rule.model} · {rule.speed}
              </div>
              {render(rule)}
            </div>
          ))}
        </div>
      )}
    </details>
  )
}
