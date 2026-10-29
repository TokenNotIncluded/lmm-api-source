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
import { Gauge, HeartPulse, Timer } from 'lucide-react'
import { useMemo } from 'react'
import { useTranslation } from 'react-i18next'

import { Badge } from '@/components/ui/badge'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { IconBadge, type IconBadgeTone } from '@/components/ui/icon-badge'
import { Separator } from '@/components/ui/separator'
import { Skeleton } from '@/components/ui/skeleton'
import { getPerfMetricsSummary } from '@/features/performance-metrics/api'
import {
  aggregatePerformanceMetric,
  isPerformancePartial,
  performanceCoverageNotice,
  formatLatency,
  formatThroughput,
  formatUptimePct,
  getSuccessRateDotClass,
  getSuccessRateTextClass,
} from '@/features/performance-metrics/lib/format'
import type { PerfModelSummary } from '@/features/performance-metrics/types'
import { cn } from '@/lib/utils'

const PERFORMANCE_WINDOW_HOURS = 24
const TOP_MODEL_LIMIT = 6

export function PerformanceOverview() {
  const { t } = useTranslation()
  const metricsQuery = useQuery({
    queryKey: ['perf-metrics-summary', PERFORMANCE_WINDOW_HOURS],
    queryFn: () => getPerfMetricsSummary(PERFORMANCE_WINDOW_HOURS),
    staleTime: 60 * 1000,
    retry: false,
  })

  const models = useMemo(
    () => metricsQuery.data?.data?.models ?? [],
    [metricsQuery.data]
  )
  const summary = useMemo(() => {
    const rows = metricsQuery.data?.data?.aggregate
      ? [metricsQuery.data.data.aggregate]
      : models
    return {
      avgLatencyMs: aggregatePerformanceMetric(rows, 'avg_latency_ms'),
      avgTps: aggregatePerformanceMetric(rows, 'avg_tps'),
      successRate: aggregatePerformanceMetric(rows, 'success_rate'),
    }
  }, [models, metricsQuery.data])
  const partial = isPerformancePartial(metricsQuery.data)
  const notice = performanceCoverageNotice(metricsQuery.data, t)
  const topModels = useMemo(() => models.slice(0, TOP_MODEL_LIMIT), [models])
  const loading = metricsQuery.isLoading
  const hasData = models.length > 0

  if (!loading && !hasData) {
    return (
      <Card size='sm'>
        <CardContent className='text-muted-foreground text-center text-xs'>
          {notice && (
            <p role='status' className='text-warning mb-2'>
              {notice}
            </p>
          )}
          {t('No performance data available')}
        </CardContent>
      </Card>
    )
  }

  return (
    <Card size='sm' className='gap-0 py-0'>
      <CardHeader className='flex flex-wrap items-center gap-x-5 gap-y-2.5 border-b px-4 py-3 sm:px-5'>
        <div className='flex items-center gap-1.5'>
          <IconBadge tone={partial ? 'warning' : 'success'} size='xs'>
            <HeartPulse />
          </IconBadge>
          <CardTitle className='text-xs whitespace-nowrap'>
            {t('Performance health')}
          </CardTitle>
        </div>

        <Separator className='hidden h-4 sm:block' orientation='vertical' />

        {loading ? (
          <div className='flex flex-wrap items-center gap-x-5 gap-y-2'>
            {['success', 'latency', 'throughput'].map((key) => (
              <div key={key} className='flex items-center gap-1.5'>
                <Skeleton className='h-3 w-14' />
                <Skeleton className='h-4 w-16' />
              </div>
            ))}
          </div>
        ) : (
          <div className='flex flex-wrap items-center gap-x-5 gap-y-2'>
            <InlineMetric
              icon={HeartPulse}
              label={t('Success rate')}
              value={formatUptimePct(summary.successRate)}
              valueClassName={
                partial
                  ? 'text-warning'
                  : getSuccessRateTextClass(summary.successRate)
              }
              tone={partial ? 'warning' : 'success'}
            />
            <InlineMetric
              icon={Timer}
              label={t('Average latency')}
              value={formatLatency(summary.avgLatencyMs)}
              tone='warning'
            />
            <InlineMetric
              icon={Gauge}
              label={t(
                metricsQuery.data?.coverage ? 'Relay throughput' : 'Throughput'
              )}
              value={formatThroughput(summary.avgTps)}
              tone='info'
            />
          </div>
        )}

        <Separator className='hidden h-4 lg:block' orientation='vertical' />
      </CardHeader>

      <CardContent className='px-4 py-3 sm:px-5'>
        {notice && (
          <p role='status' className='text-warning mb-2 text-xs'>
            {notice}
          </p>
        )}
        {!loading && hasData && (
          <div className='flex flex-wrap items-center gap-1.5'>
            {topModels.map((model) => (
              <ModelBadge
                key={model.model_name}
                model={model}
                partial={partial}
              />
            ))}
          </div>
        )}
      </CardContent>
    </Card>
  )
}

function InlineMetric(props: {
  icon: React.ComponentType<{ className?: string }>
  label: string
  value: string
  valueClassName?: string
  tone: IconBadgeTone
}) {
  const Icon = props.icon

  return (
    <div className='flex items-center gap-1.5'>
      <IconBadge tone={props.tone} size='xs'>
        <Icon />
      </IconBadge>
      <span className='text-muted-foreground text-[11px]'>{props.label}</span>
      <span
        className={cn(
          'font-mono text-xs font-semibold tabular-nums',
          props.valueClassName
        )}
      >
        {props.value}
      </span>
    </div>
  )
}

function ModelBadge(props: { model: PerfModelSummary; partial: boolean }) {
  const model = props.model

  return (
    <Badge className='max-w-full gap-1.5' variant='secondary'>
      <span className='max-w-[10rem] truncate font-mono text-[11px]'>
        {model.model_name}
      </span>
      <span
        className={cn(
          'size-1.5 rounded-full',
          props.partial
            ? 'bg-warning'
            : getSuccessRateDotClass(model.success_rate)
        )}
        aria-hidden='true'
      />
      <span
        className={cn(
          'font-mono text-[11px] font-semibold tabular-nums',
          props.partial
            ? 'text-warning'
            : getSuccessRateTextClass(model.success_rate)
        )}
      >
        {formatUptimePct(model.success_rate)}
      </span>
    </Badge>
  )
}
