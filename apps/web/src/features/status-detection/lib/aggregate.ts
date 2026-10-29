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
import { aggregatePerformanceMetric } from '@/features/performance-metrics/lib/format'
import type {
  PerformanceGroup,
  PerformanceSeriesPoint,
} from '@/features/performance-metrics/types'

import type {
  ModelPerformanceSnapshot,
  StatusGroup,
  StatusSort,
} from '../types'

type GroupAccumulator = {
  models: Set<string>
  rows: PerformanceGroup[]
  buckets: Map<number, PerformanceSeriesPoint[]>
}

/** Native rows use counters; legacy rows retain their unweighted means. */
export function aggregateStatusGroups(
  entries: ModelPerformanceSnapshot[]
): StatusGroup[] {
  const groups = new Map<string, GroupAccumulator>()
  for (const entry of entries) {
    for (const group of entry.groups) {
      const name = group.group.trim()
      if (!name) continue
      const accumulator = groups.get(name) ?? {
        models: new Set<string>(),
        rows: [],
        buckets: new Map<number, PerformanceSeriesPoint[]>(),
      }
      accumulator.models.add(entry.modelName)
      accumulator.rows.push(group)
      for (const point of group.series) {
        if (!Number.isFinite(point.ts)) continue
        const bucket = accumulator.buckets.get(point.ts) ?? []
        bucket.push(point)
        accumulator.buckets.set(point.ts, bucket)
      }
      groups.set(name, accumulator)
    }
  }
  return [...groups.entries()]
    .map(([group, accumulator]) => {
      const buckets = [...accumulator.buckets.entries()].sort(
        ([a], [b]) => a - b
      )
      const trend = (metric: 'success_rate' | 'avg_ttft_ms') =>
        buckets.flatMap(([, rows]) => {
          const value = aggregatePerformanceMetric(rows, metric)
          return value === null ? [] : [value]
        })
      return {
        group,
        avgTtftMs:
          aggregatePerformanceMetric(accumulator.rows, 'avg_ttft_ms') ??
          Number.NaN,
        avgLatencyMs:
          aggregatePerformanceMetric(accumulator.rows, 'avg_latency_ms') ??
          Number.NaN,
        avgTps:
          aggregatePerformanceMetric(accumulator.rows, 'avg_tps') ?? Number.NaN,
        successRate:
          aggregatePerformanceMetric(accumulator.rows, 'success_rate') ??
          Number.NaN,
        successTrend: trend('success_rate'),
        ttftTrend: trend('avg_ttft_ms'),
        modelCount: accumulator.models.size,
      }
    })
    .sort((left, right) => left.group.localeCompare(right.group))
}

function finitePositiveOrInfinity(value: number) {
  return Number.isFinite(value) && value > 0 ? value : Number.POSITIVE_INFINITY
}

export function sortStatusGroups(groups: StatusGroup[], sort: StatusSort) {
  return [...groups].sort((left, right) => {
    if (sort === 'name') return left.group.localeCompare(right.group)
    if (sort === 'reliability') {
      const leftRate = Number.isFinite(left.successRate)
        ? left.successRate
        : Number.NEGATIVE_INFINITY
      const rightRate = Number.isFinite(right.successRate)
        ? right.successRate
        : Number.NEGATIVE_INFINITY
      if (leftRate !== rightRate) return rightRate - leftRate
    }

    const leftTtft = finitePositiveOrInfinity(left.avgTtftMs)
    const rightTtft = finitePositiveOrInfinity(right.avgTtftMs)
    if (leftTtft !== rightTtft) return leftTtft - rightTtft
    return left.group.localeCompare(right.group)
  })
}
