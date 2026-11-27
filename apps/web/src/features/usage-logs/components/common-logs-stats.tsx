/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.

For commercial licensing, please contact support@quantumnous.com
*/
import { useQuery } from '@tanstack/react-query'
import { getRouteApi } from '@tanstack/react-router'
import { ChartNoAxesColumn, ChevronDown } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { useHistoricalQuotaMoney } from '@/hooks/use-historical-quota-money'
import { toIntlLocale } from '@/i18n/languages'
import { getCurrencyFormattingLocale } from '@/lib/currency'
import {
  formatHistoricalRawQuota,
  historicalQuotaUSD,
  type HistoricalQuotaProjection,
} from '@/lib/historical-quota-money'
import { cn } from '@/lib/utils'
import { useAuthStore } from '@/stores/auth-store'

import {
  readUsageStatistics,
  readUsageStatisticsScope,
} from '../lib/statistics'
import { formatDurationMs } from '../lib/timing'
import { buildUsageTrendBuckets } from '../lib/trend'
import { buildApiParams, getDefaultTimeRange } from '../lib/utils'
import type { LogStatistics } from '../types'
import { useLogsViewScope, useUsageLogsContext } from './usage-logs-provider'

const route = getRouteApi('/_authenticated/usage-logs/$section')

function StatBadge(props: {
  label: string
  value: string | number
  accent: string
  title?: string
}) {
  return (
    <span
      title={props.title}
      className='inline-flex min-w-0 items-baseline gap-2 py-1 text-xs'
    >
      <span className='text-muted-foreground'>{props.label}</span>
      <span className='text-foreground font-medium tabular-nums'>
        {props.value}
      </span>
    </span>
  )
}

type TrendBucket = LogStatistics & { start: number; end: number }
type CompleteTrendBucket = TrendBucket & { quota: number }

function statisticsProjection(
  statistics: LogStatistics | undefined
): HistoricalQuotaProjection | undefined {
  return statistics
    ? { ...statistics, quota_usd: statistics.quota_usd ?? undefined }
    : undefined
}

function hasCompleteTrendQuota(
  bucket: TrendBucket
): bucket is CompleteTrendBucket {
  return (
    bucket.quota !== null &&
    (bucket.invalid_quota_records ?? 0) === 0 &&
    bucket.quota_projection_truncated !== true
  )
}

function UsageTrend(props: {
  buckets: TrendBucket[]
  loading: boolean
  error: boolean
  onRetry: () => void
  onSelect: (bucket: TrendBucket) => void
}) {
  const { t, i18n } = useTranslation()
  const { formatHistoricalQuota, formatRawQuota } = useHistoricalQuotaMoney()

  if (props.loading) {
    return <Skeleton className='h-16 w-full min-w-[220px] rounded-md' />
  }
  if (props.error) {
    return (
      <div className='text-muted-foreground flex items-center gap-2 text-xs'>
        <span>{t('Unable to load usage trend')}</span>
        <button
          type='button'
          className='text-foreground underline underline-offset-2'
          onClick={props.onRetry}
        >
          {t('Retry')}
        </button>
      </div>
    )
  }
  const buckets = props.buckets.filter(hasCompleteTrendQuota)
  if (buckets.length === 0 || buckets.length !== props.buckets.length) {
    return (
      <span className='text-muted-foreground text-xs'>
        {t('Usage trend for the selected time range')}: {t('Unavailable')}
      </span>
    )
  }
  const moneyAvailable = buckets.every(
    (bucket) => historicalQuotaUSD(statisticsProjection(bucket)) !== null
  )
  const metric = (bucket: CompleteTrendBucket) =>
    moneyAvailable
      ? (historicalQuotaUSD(statisticsProjection(bucket)) ?? bucket.quota)
      : bucket.quota
  const formatBucketQuota = (bucket: CompleteTrendBucket) =>
    moneyAvailable
      ? formatHistoricalQuota(bucket.quota, statisticsProjection(bucket))
      : formatRawQuota(bucket.quota)
  const maxMetric = Math.max(Number.EPSILON, ...buckets.map(metric))
  if (
    !buckets.some(
      (bucket) =>
        bucket.quota > 0 ||
        (bucket.rpm !== null && bucket.rpm > 0) ||
        (bucket.tpm !== null && bucket.tpm > 0)
    )
  ) {
    return (
      <span className='text-muted-foreground text-xs'>
        {t('No usage trend data in this range')}
      </span>
    )
  }

  const formatBucket = (timestamp: number) =>
    new Intl.DateTimeFormat(
      toIntlLocale(i18n.resolvedLanguage || i18n.language),
      {
        month: 'short',
        day: 'numeric',
        hour: 'numeric',
      }
    ).format(new Date(timestamp))

  return (
    <div className='min-w-[220px] flex-1'>
      <div
        className='flex h-16 items-end gap-1'
        role='group'
        aria-label={t('Usage trend for the selected time range')}
      >
        {buckets.map((bucket) => {
          const value = metric(bucket)
          const height = value > 0 ? Math.max(10, (value / maxMetric) * 100) : 4
          return (
            <button
              key={`${bucket.start}-${bucket.end}`}
              type='button'
              className='bg-primary/55 hover:bg-primary/80 focus-visible:ring-ring min-w-0 flex-1 rounded-t-sm transition-[height,background-color] focus-visible:ring-2 focus-visible:outline-none motion-reduce:transition-none'
              style={{ height: `${height}%` }}
              title={`${formatBucket(bucket.start)} · ${formatBucketQuota(bucket)}`}
              aria-label={`${formatBucket(bucket.start)}: ${formatBucketQuota(bucket)}`}
              onClick={() => props.onSelect(bucket)}
            />
          )
        })}
      </div>
      <p className='text-muted-foreground mt-1 text-[11px]'>
        {t('Click a bar to filter logs to that time window.')}
        {!moneyAvailable
          ? ` ${t('Historical monetary value unavailable')}`
          : ''}
      </p>
    </div>
  )
}

export function CommonLogsStats({
  refreshInterval = false,
}: {
  refreshInterval?: number | false
}) {
  const { t, i18n } = useTranslation()
  const { formatHistoricalQuota, formatRawQuota } = useHistoricalQuotaMoney()
  const { isAdminView: isAdmin, historical } = useLogsViewScope()
  const searchParams = route.useSearch()
  const { sensitiveVisible } = useUsageLogsContext()
  const navigate = route.useNavigate()
  const auth = useAuthStore((state) => state.auth)
  const scope = readUsageStatisticsScope(auth)
  const scopeKey = [scope?.userId, scope?.sessionId, scope?.role]
  const [trendOpen, setTrendOpen] = useState(false)
  const locale = toIntlLocale(i18n.resolvedLanguage || i18n.language)
  const number = (value: number | null | undefined) =>
    value == null
      ? '—'
      : value.toLocaleString(locale, { maximumFractionDigits: 2 })

  const statsQuery = useQuery({
    queryKey: [
      'usage-logs-stats',
      ...scopeKey,
      isAdmin,
      historical,
      searchParams,
    ],
    queryFn: async ({ signal }) => {
      if (!scope) throw new Error('Usage statistics session unavailable')
      const params = buildApiParams({
        page: 1,
        pageSize: 1,
        searchParams,
        columnFilters: [],
        isAdmin,
        historical,
      })
      return readUsageStatistics(scope, params, isAdmin, signal, historical)
    },
    enabled: !!scope,
    retry: false,
    refetchInterval: (query) =>
      query.state.status === 'error' ? false : refreshInterval,
    refetchIntervalInBackground: false,
  })

  const trendQuery = useQuery({
    queryKey: [
      'usage-logs-trend',
      ...scopeKey,
      isAdmin,
      historical,
      searchParams,
    ],
    queryFn: async ({ signal }) => {
      if (!scope) throw new Error('Usage statistics session unavailable')
      const fallback = getDefaultTimeRange()
      const start = Number(searchParams.startTime) || fallback.start.getTime()
      const end = Number(searchParams.endTime) || Date.now()
      const buckets = buildUsageTrendBuckets(start, end)
      return Promise.all(
        buckets.map(async (bucket) => {
          const params = buildApiParams({
            page: 1,
            pageSize: 1,
            searchParams: {
              ...searchParams,
              startTime: bucket.start,
              endTime: bucket.end,
            },
            columnFilters: [],
            isAdmin,
          })
          const result = await readUsageStatistics(
            scope,
            params,
            isAdmin,
            signal,
            historical
          )
          return { ...bucket, ...result }
        })
      )
    },
    enabled: trendOpen && sensitiveVisible && !!scope,
    retry: false,
  })

  const stats = statsQuery.isError ? undefined : statsQuery.data
  const native = stats?.source !== 'legacy_log_only' ? stats : undefined
  const statisticsUnavailable = !scope || (!stats && !statsQuery.isLoading)
  const quotaLabel = stats
    ? stats.quota !== null
      ? formatHistoricalQuota(stats.quota, statisticsProjection(stats))
      : formatHistoricalRawQuota(
          stats.raw_consumption_quota,
          t('Raw credits'),
          getCurrencyFormattingLocale(i18n.resolvedLanguage || i18n.language)
        )
    : '—'
  const secondaryMetrics = [
    { label: t('Success'), value: native?.success_count },
    { label: t('Pending'), value: native?.pending_count },
    { label: t('Refunded'), value: native?.refunded_count },
    { label: t('Total Tokens'), value: native?.total_tokens },
    { label: t('Input Tokens'), value: native?.input_tokens },
    { label: t('Output Tokens'), value: native?.output_tokens },
    { label: t('Selected range RPM'), value: native?.requests_per_minute },
    { label: t('Selected range TPM'), value: native?.tokens_per_minute },
    { label: t('Cache read tokens'), value: native?.cache_read_tokens },
    { label: t('Cache write tokens'), value: native?.cache_write_tokens },
    { label: t('Cache Write (5m)'), value: native?.cache_write_5m_tokens },
    { label: t('Cache Write (1h)'), value: native?.cache_write_1h_tokens },
    {
      label: t('Cache write (12 hours)'),
      value: native?.cache_write_12h_tokens,
    },
    {
      label: t('Images (billing quantity)'),
      value: native?.output_image_count,
    },
    { label: t('Successful tool requests'), value: native?.tool_requests },
    { label: t('Completed text requests'), value: native?.completed_requests },
    { label: t('Grounded requests'), value: native?.grounded_requests },
    { label: t('Incomplete'), value: native?.incomplete_count },
  ].filter((metric) => metric.value !== undefined)

  return (
    <section
      className='min-w-0 space-y-3 py-1'
      aria-label={t('Usage statistics')}
      aria-busy={statsQuery.isFetching}
    >
      <div className='grid grid-cols-2 gap-x-6 gap-y-4 lg:grid-cols-4'>
        {[
          {
            label: t('Usage'),
            value: sensitiveVisible ? quotaLabel : '••••',
            title:
              sensitiveVisible &&
              historicalQuotaUSD(statisticsProjection(stats)) === null
                ? t('Historical monetary value unavailable')
                : undefined,
          },
          { label: t('Matching requests'), value: number(native?.total_count) },
          {
            label: t('Failed requests'),
            value: number(native?.error_count),
            danger: (native?.error_count ?? 0) > 0,
          },
          {
            label: t('P95 duration'),
            value: formatDurationMs(native?.p95_duration_ms),
            title: t(
              'Calculated from recorded request durations in the selected range.'
            ),
          },
        ].map((metric) => (
          <div
            key={metric.label}
            className='min-w-0 space-y-1'
            title={metric.title}
          >
            <p className='text-muted-foreground text-xs'>{metric.label}</p>
            {statsQuery.isLoading ? (
              <Skeleton className='h-7 w-28' />
            ) : (
              <p
                className={cn(
                  'break-words text-lg font-medium tracking-tight tabular-nums sm:text-xl',
                  metric.danger && 'text-destructive'
                )}
              >
                {metric.value}
              </p>
            )}
          </div>
        ))}
      </div>
      <div className='text-muted-foreground flex min-w-0 flex-wrap items-center gap-x-5 gap-y-1 text-xs'>
        <span>{t('Last 60 seconds')}</span>
        <StatBadge label={t('RPM')} value={stats?.rpm ?? '—'} accent='' />
        <StatBadge
          label={t('TPM')}
          value={stats?.tpm ?? '—'}
          accent=''
          title={
            native?.token_count_scope === 'settled_text_requests'
              ? t('Tokens cover settled text requests only.')
              : undefined
          }
        />
        {statisticsUnavailable && (
          <span role='alert' className='flex items-center gap-2'>
            {t('Failed to load')}
            <button
              type='button'
              className='text-foreground underline underline-offset-2'
              disabled={!scope}
              onClick={() => {
                void statsQuery.refetch()
                if (trendOpen && sensitiveVisible) void trendQuery.refetch()
              }}
            >
              {t('Retry')}
            </button>
          </span>
        )}
        <Button
          type='button'
          variant='ghost'
          size='sm'
          className='ms-auto gap-1.5'
          aria-expanded={trendOpen}
          disabled={!sensitiveVisible}
          onClick={() => setTrendOpen((open) => !open)}
        >
          <ChartNoAxesColumn className='size-3.5' aria-hidden='true' />
          {t('Usage trend')}
          <ChevronDown
            className={cn('size-3.5', trendOpen && 'rotate-180')}
            aria-hidden='true'
          />
        </Button>
      </div>
      {secondaryMetrics.length > 0 && (
        <details className='group min-w-0'>
          <summary className='text-muted-foreground hover:text-foreground w-fit cursor-pointer text-xs focus-visible:outline-offset-4'>
            {t('Usage breakdown')}
          </summary>
          <div className='mt-3 grid max-h-[30vh] grid-cols-2 gap-x-6 gap-y-1 overflow-y-auto md:grid-cols-3 xl:grid-cols-4'>
            {secondaryMetrics.map((metric) => (
              <StatBadge
                key={metric.label}
                label={metric.label}
                value={number(metric.value)}
                accent=''
              />
            ))}
            {native?.average_duration_ms !== undefined && (
              <StatBadge
                label={t('Average duration')}
                value={formatDurationMs(native.average_duration_ms)}
                accent=''
              />
            )}
            {native?.duration_sample_count !== undefined && (
              <StatBadge
                label={t('Recorded durations')}
                value={number(native.duration_sample_count)}
                accent=''
              />
            )}
            {native?.reserved_credits !== undefined && (
              <StatBadge
                label={t('Reserved funds')}
                value={
                  sensitiveVisible
                    ? formatRawQuota(native.reserved_credits)
                    : '••••'
                }
                accent=''
              />
            )}
            {native?.platform_sponsored_quota !== undefined && (
              <StatBadge
                label={t('Platform-sponsored usage')}
                value={
                  sensitiveVisible
                    ? formatRawQuota(native.platform_sponsored_quota)
                    : '••••'
                }
                accent=''
              />
            )}
          </div>
          <p className='text-muted-foreground mt-2 text-xs'>
            {t(
              'Range rates use the selected time window. Live rates use the last 60 seconds.'
            )}
          </p>
        </details>
      )}
      {trendOpen && sensitiveVisible && !statisticsUnavailable && (
        <div className='py-2'>
          <UsageTrend
            buckets={trendQuery.data ?? []}
            loading={trendQuery.isLoading || trendQuery.isFetching}
            error={trendQuery.isError}
            onRetry={() => void trendQuery.refetch()}
            onSelect={(bucket) =>
              void navigate({
                to: '/usage-logs/$section',
                params: { section: 'common' },
                search: (previous) => ({
                  ...previous,
                  startTime: bucket.start,
                  endTime: bucket.end,
                  page: 1,
                }),
              })
            }
          />
        </div>
      )}
    </section>
  )
}
