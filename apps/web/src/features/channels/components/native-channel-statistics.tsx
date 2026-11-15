/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useId } from 'react'
import { useTranslation } from 'react-i18next'

import type {
  NativeChannelSort,
  NativeChannelSortInput,
  NativeChannelStatistics,
  NativeChannelSummary,
} from '../native-channel-statistics'

export function NativeChannelSortFields({
  value,
  onChange,
  units,
  disabled,
}: {
  disabled: boolean
  units: string[]
  value: NativeChannelSortInput
  onChange: (value: NativeChannelSortInput) => void
}) {
  const { t } = useTranslation()
  const id = useId()
  return (
    <div className='flex flex-wrap gap-3'>
      <label htmlFor={`${id}-sort`} className='space-y-1 text-sm'>
        <span>{t('Sort by')}</span>
        <select
          disabled={disabled}
          id={`${id}-sort`}
          value={value.sort}
          className='border-input bg-background block h-11 rounded-md border px-3'
          onChange={(e) =>
            onChange({
              ...value,
              sort: e.target.value as NativeChannelSort,
              page: 1,
            })
          }
        >
          {(
            [
              ['id', 'ID'],
              ['name', 'Name'],
              ['priority', 'Priority'],
              ['lifetime', 'channelCounters.lifetime'],
              ['balance', 'Balance'],
              ['response_time', 'channelSorting.catalogDuration'],
              ['test_time', 'channelSorting.catalogTime'],
            ] as const
          ).map(([key, label]) => (
            <option key={key} value={key}>
              {t(label)}
            </option>
          ))}
        </select>
      </label>
      <label htmlFor={`${id}-order`} className='space-y-1 text-sm'>
        <span>{t('Sort order')}</span>
        <select
          disabled={disabled}
          id={`${id}-order`}
          value={value.order}
          className='border-input bg-background block h-11 rounded-md border px-3'
          onChange={(e) =>
            onChange({
              ...value,
              order: e.target.value as 'asc' | 'desc',
              page: 1,
            })
          }
        >
          <option value='asc'>{t('Ascending')}</option>
          <option value='desc'>{t('Descending')}</option>
        </select>
      </label>
      {value.sort === 'balance' && (
        <label htmlFor={`${id}-unit`} className='space-y-1 text-sm'>
          <span>{t('channelSorting.balanceUnit')}</span>
          <select
            disabled={disabled}
            id={`${id}-unit`}
            value={value.unit}
            className='border-input bg-background block h-11 rounded-md border px-3'
            onChange={(e) =>
              onChange({ ...value, unit: e.target.value, page: 1 })
            }
          >
            <option value=''>{t('channelSorting.balanceUnit')}</option>
            {[...new Set([...units, ...(value.unit ? [value.unit] : [])])].map(
              (unit) => (
                <option key={unit} value={unit}>
                  {unit}
                </option>
              )
            )}
          </select>
        </label>
      )}
    </div>
  )
}

export function NativeChannelOperationsSummary({
  value,
}: {
  value: NativeChannelSummary
}) {
  const { t } = useTranslation()
  const metrics = [
    { label: t('Total'), value: value.total },
    { label: t('Enabled'), value: value.enabled },
    { label: t('Success'), value: value.successful },
    { label: t('Failed'), value: value.failed },
    { label: t('Untested'), value: value.untested },
    { label: t('Slow (> 5s)'), value: value.slow_over_5000ms },
    { label: t('Zero balances'), value: value.zero_balance },
    { label: t('Unknown balances'), value: value.unknown_balance },
    {
      label: t('Average duration'),
      value:
        value.average_response_time_ms === null
          ? t('Unknown')
          : `${value.average_response_time_ms} ms`,
    },
  ]
  return (
    <section
      className='bg-card space-y-3 rounded-lg p-4'
      aria-label={t('Summary')}
    >
      <h3 className='text-sm font-medium'>{t('Summary')}</h3>
      <dl className='grid grid-cols-2 gap-3 sm:grid-cols-3 xl:grid-cols-5'>
        {metrics.map((metric) => (
          <div key={metric.label} className='min-w-0'>
            <dt className='text-muted-foreground text-xs'>{metric.label}</dt>
            <dd className='text-lg font-medium tabular-nums'>{metric.value}</dd>
          </div>
        ))}
      </dl>
    </section>
  )
}

export function NativeChannelStatisticsRow({
  value,
}: {
  value: NativeChannelStatistics | undefined
}) {
  const { t } = useTranslation()
  if (!value) return null
  const balance = value.observation
  const probe = value.catalog_observation
  return (
    <div className='space-y-1 text-sm'>
      <p>
        {t('channelCounters.lifetime')}:{' '}
        {value.lifetime.complete
          ? `${value.lifetime.total_usd} USD`
          : t('Unknown')}
      </p>
      <p>
        {t('Balance')}:{' '}
        {balance?.balance === null || balance?.balance === undefined
          ? t('Unknown')
          : `${balance.balance} ${balance.unit_label ?? t('channelBalance.unitUnknown')}`}
      </p>
      {balance && (
        <p>
          {t('channelBalance.source')}:{' '}
          {t(
            `channelBalance.${balance.kind === 'provider' ? 'provider' : balance.kind === 'copied' ? 'copied' : balance.kind === 'reset' ? 'reset' : 'unknown'}`
          )}{' '}
          · {t('channelSorting.balanceObservedAt')}:{' '}
          {balance.balance_updated_time === null
            ? t('Unknown')
            : new Date(balance.balance_updated_time * 1000).toLocaleString()}
        </p>
      )}
      <p>
        {t('channelSorting.catalogDuration')}:{' '}
        {value.response_time === null
          ? t('Unknown')
          : `${value.response_time} ms`}
      </p>
      <p>
        {t('channelSorting.catalogTime')}:{' '}
        {probe
          ? new Date(probe.observed_at * 1000).toLocaleString()
          : t('Unknown')}{' '}
        {probe && `· ${t(probe.result === 'success' ? 'Success' : 'Failed')}`}
      </p>
      <p className='text-muted-foreground text-xs'>
        {t('channelSorting.catalogNote')}
      </p>
    </div>
  )
}
