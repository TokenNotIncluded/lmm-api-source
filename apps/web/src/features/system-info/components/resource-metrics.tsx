/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import { Badge } from '@/components/ui/badge'
import { toIntlLocale } from '@/i18n/languages'

import type { SystemResourceProcess, SystemResourceSnapshot } from '../types'

export function ResourceMetric({
  label,
  value,
}: {
  label: string
  value: ReactNode
}) {
  return (
    <div className='flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1 text-sm'>
      <dt className='text-muted-foreground min-w-0'>{label}</dt>
      <dd className='min-w-0 text-right break-words tabular-nums'>{value}</dd>
    </div>
  )
}

function useResourceFormat() {
  const { t, i18n } = useTranslation()
  const locale = toIntlLocale(i18n.language)
  const number = new Intl.NumberFormat(locale, { maximumFractionDigits: 2 })
  const percentage = new Intl.NumberFormat(locale, {
    style: 'percent',
    maximumFractionDigits: 1,
  })
  return {
    number: (value: number | undefined) =>
      value === undefined ? t('Not reported') : number.format(value),
    percent: (value: number | undefined) =>
      value === undefined ? t('Not reported') : percentage.format(value / 100),
    bytes: (value: number | undefined) => {
      if (value === undefined) return t('Not reported')
      const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB']
      const index =
        value === 0 ? 0 : Math.min(4, Math.floor(Math.log2(value) / 10))
      return `${number.format(value / 1024 ** index)} ${units[index]}`
    },
  }
}

export function HostResources({
  snapshot,
}: {
  snapshot: SystemResourceSnapshot
}) {
  const { t } = useTranslation()
  const format = useResourceFormat()
  const { system } = snapshot
  return (
    <div className='space-y-6' data-resource='host'>
      <div className='space-y-3'>
        <div className='flex flex-wrap items-baseline justify-between gap-2'>
          <h4 className='text-sm font-medium'>{t('CPU')}</h4>
          <span className='text-2xl font-semibold tracking-tight tabular-nums'>
            {format.percent(system.cpu_usage_percent)}
          </span>
        </div>
        <dl className='space-y-2'>
          <ResourceMetric
            label={t('Logical CPUs')}
            value={format.number(system.logical_cpus)}
          />
          <ResourceMetric
            label={t('Load average (1 / 5 / 15 min)')}
            value={[
              system.load_average.one,
              system.load_average.five,
              system.load_average.fifteen,
            ]
              .map(format.number)
              .join(' / ')}
          />
        </dl>
      </div>
      <div className='space-y-3'>
        <div className='flex flex-wrap items-baseline justify-between gap-2'>
          <h4 className='text-sm font-medium'>{t('Memory')}</h4>
          <span className='text-2xl font-semibold tracking-tight tabular-nums'>
            {format.percent(system.memory_usage_percent)}
          </span>
        </div>
        <dl className='space-y-2'>
          <ResourceMetric
            label={t('Total')}
            value={format.bytes(system.memory_total_bytes)}
          />
          <ResourceMetric
            label={t('Used')}
            value={format.bytes(system.memory_used_bytes)}
          />
          <ResourceMetric
            label={t('Available')}
            value={format.bytes(system.memory_available_bytes)}
          />
        </dl>
      </div>
    </div>
  )
}

export function ProcessResources({
  label,
  process,
}: {
  label: string
  process: SystemResourceProcess
}) {
  const { t } = useTranslation()
  const format = useResourceFormat()
  return (
    <div className='space-y-3' data-process-status={process.status}>
      <h4 className='text-sm font-medium'>{label}</h4>
      <div className='flex flex-wrap items-center gap-x-3 gap-y-2'>
        <Badge variant={process.status === 'running' ? 'success' : 'secondary'}>
          {t(process.status === 'running' ? 'Running' : 'Unavailable')}
        </Badge>
        <span className='text-muted-foreground text-xs break-all'>
          {t('Version')}: {process.version ?? t('Not reported')}
        </span>
      </div>
      {process.status === 'running' ? (
        <dl className='space-y-2'>
          <ResourceMetric
            label={t('PID')}
            value={process.pid ?? t('Not reported')}
          />
          <ResourceMetric
            label={t('CPU')}
            value={format.percent(process.cpu_percent)}
          />
          <ResourceMetric
            label={t('RSS')}
            value={format.bytes(process.memory_bytes)}
          />
          <ResourceMetric
            label={t('Memory')}
            value={format.percent(process.memory_percent)}
          />
          <ResourceMetric
            label={t('Threads')}
            value={format.number(process.threads)}
          />
        </dl>
      ) : (
        <p className='text-muted-foreground text-sm'>{t('Not reported')}</p>
      )}
    </div>
  )
}

export function ResourceSampleMetadata({
  snapshot,
}: {
  snapshot: SystemResourceSnapshot
}) {
  const { t, i18n } = useTranslation()
  const locale = toIntlLocale(i18n.language)
  // Core uses zero when the host clock cannot provide a Unix timestamp.
  const date =
    snapshot.observed_at_unix_ms === 0
      ? undefined
      : new Date(snapshot.observed_at_unix_ms)
  const sampledAt =
    date &&
    new Intl.DateTimeFormat(locale, {
      dateStyle: 'medium',
      timeStyle: 'medium',
    }).format(date)
  const duration = new Intl.NumberFormat(locale, {
    style: 'unit',
    unit: 'millisecond',
    maximumFractionDigits: 0,
  }).format(snapshot.sample_window_ms)
  return (
    <dl className='text-muted-foreground space-y-2 pt-2 text-xs'>
      <div className='space-y-1'>
        <dt>{t('Sampled at')}</dt>
        <dd>
          {date ? (
            <time dateTime={date.toISOString()}>{sampledAt}</time>
          ) : (
            t('Not reported')
          )}
        </dd>
      </div>
      <div className='flex flex-wrap items-baseline gap-x-3 gap-y-1'>
        <dt>{t('Sample window')}</dt>
        <dd className='tabular-nums'>{duration}</dd>
      </div>
    </dl>
  )
}
