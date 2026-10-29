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
/*
Copyright (C) 2026 LIghtJUNction
*/
import {
  aggregatePerformanceMetric,
  type PerformanceMetric,
} from '@/features/performance-metrics/lib/format'
import type {
  PerformanceGroup,
  PerformanceSeriesPoint,
} from '@/features/performance-metrics/types'

export type LatencyTimePoint = {
  timestamp: string
  group: string
  ttft_ms: number
}

export type SuccessRateTimePoint = {
  timestamp: string
  success_rate: number
}

// Align observed UTC buckets, then combine their actual counters.
function aggregateSeries(
  groups: PerformanceGroup[],
  metric: PerformanceMetric
) {
  const buckets = new Map<number, PerformanceSeriesPoint[]>()
  for (const group of groups) {
    for (const point of group.series) {
      if (
        !Number.isFinite(point.ts) ||
        !Number.isFinite(new Date(point.ts * 1000).getTime())
      ) {
        continue
      }
      const bucket = buckets.get(point.ts) ?? []
      bucket.push(point)
      buckets.set(point.ts, bucket)
    }
  }
  return [...buckets.entries()]
    .sort(([a], [b]) => a - b)
    .flatMap(([ts, rows]) => {
      const value = aggregatePerformanceMetric(rows, metric)
      return value === null
        ? []
        : [{ timestamp: new Date(ts * 1000).toISOString(), value }]
    })
}

export function toLatencySeries(
  groups: PerformanceGroup[]
): LatencyTimePoint[] {
  return aggregateSeries(groups, 'avg_ttft_ms').map(({ timestamp, value }) => ({
    timestamp,
    group: 'latency',
    ttft_ms: Math.round(value),
  }))
}

export function toSuccessRateSeries(
  groups: PerformanceGroup[]
): SuccessRateTimePoint[] {
  return aggregateSeries(groups, 'success_rate').map(
    ({ timestamp, value }) => ({
      timestamp,
      success_rate: Math.round(value * 100) / 100,
    })
  )
}
