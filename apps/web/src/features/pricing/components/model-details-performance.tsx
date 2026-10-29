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
import { HeartPulse, Timer } from 'lucide-react'
import { useMemo } from 'react'
import { useTranslation } from 'react-i18next'

import {
  StaticDataTable,
  staticDataTableClassNames as tableStyles,
} from '@/components/data-table'
import { GroupBadge } from '@/components/group-badge'
import { getPerfMetrics } from '@/features/performance-metrics/api'
import {
  aggregatePerformanceMetric,
  isPerformancePartial,
  performanceCoverageNotice,
  RELAY_TIMING_DESCRIPTION,
  formatLatency,
  formatThroughput,
  formatUptimePct,
  getSuccessRateTextClass,
} from '@/features/performance-metrics/lib/format'
import { cn } from '@/lib/utils'

import {
  toLatencySeries,
  toSuccessRateSeries,
  type SuccessRateTimePoint,
} from '../lib/performance-series'
import type { PricingModel } from '../types'
import {
  LatencyTrendChart,
  SuccessRateTrendChart,
} from './model-details-charts'
import { SuccessRateSparkline } from './model-details-success-rate-sparkline'

function StatCard(props: {
  icon: React.ComponentType<{ className?: string }>
  label: string
  value: React.ReactNode
  hint?: string
  valueClassName?: string
}) {
  const Icon = props.icon
  return (
    <div className='bg-background flex flex-col gap-1 rounded-lg border p-3'>
      <span className='text-muted-foreground inline-flex items-center gap-1.5 text-[10px] font-medium tracking-wider uppercase'>
        <Icon className='size-3' />
        {props.label}
      </span>
      <span
        className={cn(
          'text-foreground font-mono text-lg font-semibold tabular-nums',
          props.valueClassName
        )}
      >
        {props.value}
      </span>
      {props.hint && (
        <span className='text-muted-foreground/70 text-[11px]'>
          {props.hint}
        </span>
      )}
    </div>
  )
}

export function ModelDetailsPerformance(props: { model: PricingModel }) {
  const { t } = useTranslation()
  const metricsQuery = useQuery({
    queryKey: ['perf-metrics', props.model.model_name],
    queryFn: () => getPerfMetrics(props.model.model_name, 24),
    staleTime: 60 * 1000,
    enabled: props.model.performance_available !== false,
  })
  const groups = useMemo(
    () =>
      props.model.performance_available === false
        ? []
        : (metricsQuery.data?.data.groups ?? []),
    [metricsQuery.data, props.model.performance_available]
  )
  const performances = groups
  const partial = isPerformancePartial(metricsQuery.data)
  const notice = performanceCoverageNotice(metricsQuery.data, t)
  const latencySeries = useMemo(() => toLatencySeries(groups), [groups])
  const successRateSeries = useMemo(() => toSuccessRateSeries(groups), [groups])
  const successRateByGroup = useMemo<
    Record<string, SuccessRateTimePoint[]>
  >(() => {
    const map: Record<string, SuccessRateTimePoint[]> = {}
    for (const group of groups) {
      map[group.group] = toSuccessRateSeries([group])
    }
    return map
  }, [groups])

  if (metricsQuery.isLoading || performances.length === 0) {
    return (
      <div className='text-muted-foreground rounded-lg border p-6 text-center text-sm'>
        {notice && (
          <p role='status' className='text-warning mb-2'>
            {notice}
          </p>
        )}
        {t('Performance data is not yet available for this model.')}
      </div>
    )
  }

  const avgTps = aggregatePerformanceMetric(groups, 'avg_tps')
  const avgLatency = aggregatePerformanceMetric(groups, 'avg_latency_ms')
  const successRate = aggregatePerformanceMetric(groups, 'success_rate')

  return (
    <div className='flex flex-col gap-4'>
      {notice && (
        <p role='status' className='text-warning text-sm'>
          {notice}
        </p>
      )}
      {metricsQuery.data?.coverage && (
        <p className='text-muted-foreground text-xs'>
          {t(RELAY_TIMING_DESCRIPTION)}
        </p>
      )}
      <div className='grid grid-cols-1 gap-2 sm:grid-cols-3'>
        <StatCard
          icon={Timer}
          label={t(
            metricsQuery.data?.coverage ? 'Relay throughput' : 'Throughput'
          )}
          value={formatThroughput(avgTps)}
          hint={
            metricsQuery.data?.coverage
              ? undefined
              : t('Sustained tokens per second')
          }
        />
        <StatCard
          icon={Timer}
          label={t('Average latency')}
          value={formatLatency(avgLatency)}
        />
        <StatCard
          icon={HeartPulse}
          label={t('Success rate')}
          value={formatUptimePct(successRate)}
          valueClassName={
            partial ? 'text-warning' : getSuccessRateTextClass(successRate)
          }
        />
      </div>

      <section>
        <SectionHeader
          icon={HeartPulse}
          title={t('Per-group performance')}
          description={t('Average latency, TTFT, TPS, and success rate')}
        />
        <StaticDataTable
          className='rounded-lg'
          tableClassName='text-sm'
          headerRowClassName={tableStyles.compactHeaderRow}
          data={performances}
          getRowKey={(perf) => perf.group}
          columns={[
            {
              id: 'group',
              header: t('Group'),
              className: tableStyles.compactHeaderCell,
              cellClassName: tableStyles.compactCell,
              cell: (perf) => <GroupBadge group={perf.group} size='sm' />,
            },
            {
              id: 'tps',
              header: 'TPS',
              className: tableStyles.compactHeaderCellRight,
              cellClassName: tableStyles.compactNumericCell,
              cell: (perf) => formatThroughput(perf.avg_tps),
            },
            {
              id: 'ttft',
              header: t('Average TTFT'),
              className: tableStyles.compactHeaderCellRight,
              cellClassName: tableStyles.compactNumericCell,
              cell: (perf) => formatLatency(perf.avg_ttft_ms),
            },
            {
              id: 'latency',
              header: t('Average latency'),
              className: tableStyles.compactHeaderCellRight,
              cellClassName: tableStyles.compactMutedNumericCell,
              cell: (perf) => formatLatency(perf.avg_latency_ms),
            },
            {
              id: 'success',
              header: t('Success rate'),
              className: cn(tableStyles.compactHeaderCell, 'min-w-[180px]'),
              cellClassName: tableStyles.compactCell,
              cell: (perf) => (
                <SuccessRateSparkline
                  size='sm'
                  overallRate={perf.success_rate}
                  partial={partial}
                  series={successRateByGroup[perf.group] ?? []}
                />
              ),
            },
          ]}
        />
      </section>

      <section>
        <SectionHeader
          icon={Timer}
          title={t('Latency trend (last 24h)')}
          description={t('Average TTFT')}
        />
        <LatencyTrendChart series={latencySeries} />
      </section>

      <section>
        <SectionHeader
          icon={HeartPulse}
          title={t('Success rate')}
          description={t('Request success rate sampled over the last 24 hours')}
        />
        <SuccessRateTrendChart series={successRateSeries} />
      </section>
    </div>
  )
}

function SectionHeader(props: {
  icon: React.ComponentType<{ className?: string }>
  title: string
  description?: string
}) {
  const Icon = props.icon
  return (
    <div className='mb-2 flex flex-wrap items-center justify-between gap-2'>
      <div className='flex min-w-0 items-center gap-2'>
        <Icon className='text-muted-foreground/70 size-3.5 shrink-0' />
        <div className='min-w-0'>
          <div className='text-foreground text-sm font-semibold'>
            {props.title}
          </div>
          {props.description && (
            <p className='text-muted-foreground/80 text-xs'>
              {props.description}
            </p>
          )}
        </div>
      </div>
    </div>
  )
}
