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
import { resolveForgeColor } from '@/lib/forge-colors'

import type {
  PerformanceAvailability,
  PerformanceCounters,
  PerformanceStatistics,
} from '../types'

export function formatThroughput(tps: number | null | undefined): string {
  if (tps == null || !Number.isFinite(tps) || tps < 0) return '—'
  if (tps >= 1_000) return `${(tps / 1_000).toFixed(1)}K t/s`
  return `${tps.toFixed(tps < 10 ? 2 : 1)} t/s`
}

export function formatLatency(ms: number | null | undefined): string {
  if (ms == null || !Number.isFinite(ms) || ms < 0) return '—'
  if (ms >= 1_000) return `${(ms / 1_000).toFixed(2)}s`
  return `${Math.round(ms)}ms`
}

export function formatUptimePct(pct: number | null | undefined): string {
  if (pct == null || !Number.isFinite(pct)) return '—'
  return `${pct.toFixed(2)}%`
}

export type SuccessRateLevel =
  | 'excellent'
  | 'good'
  | 'warning'
  | 'critical'
  | 'unknown'

const SUCCESS_RATE_EXCELLENT_MIN = 100
const SUCCESS_RATE_GOOD_MIN = 90
const SUCCESS_RATE_WARNING_MIN = 70

/**
 * Single source of truth for grading a success rate (0-100).
 * - excellent: 100% (full green)
 * - good: >= 90% (slightly lighter green)
 * - warning: >= 70%
 * - critical: below 70%
 * - unknown: non-finite values
 */
export function getSuccessRateLevel(
  rate: number | null | undefined
): SuccessRateLevel {
  if (rate == null || !Number.isFinite(rate)) return 'unknown'
  if (rate >= SUCCESS_RATE_EXCELLENT_MIN) return 'excellent'
  if (rate >= SUCCESS_RATE_GOOD_MIN) return 'good'
  if (rate >= SUCCESS_RATE_WARNING_MIN) return 'warning'
  return 'critical'
}

const SUCCESS_RATE_TEXT_CLASS: Record<SuccessRateLevel, string> = {
  excellent:
    'text-[var(--forge-chart-success-light)] dark:text-[var(--forge-chart-success-dark)]',
  good: 'text-[var(--forge-chart-good-light)] dark:text-[var(--forge-chart-good-dark)]',
  warning:
    'text-[var(--forge-chart-warning-light)] dark:text-[var(--forge-chart-warning-dark)]',
  critical:
    'text-[var(--forge-chart-critical-light)] dark:text-[var(--forge-chart-critical-dark)]',
  unknown: 'text-muted-foreground',
}

const SUCCESS_RATE_DOT_CLASS: Record<SuccessRateLevel, string> = {
  excellent:
    'bg-[var(--forge-chart-success-light)] dark:bg-[var(--forge-chart-success-dark)]',
  good: 'bg-[var(--forge-chart-good-light)] dark:bg-[var(--forge-chart-good-dark)]',
  warning:
    'bg-[var(--forge-chart-warning-light)] dark:bg-[var(--forge-chart-warning-dark)]',
  critical:
    'bg-[var(--forge-chart-critical-light)] dark:bg-[var(--forge-chart-critical-dark)]',
  unknown: 'bg-muted-foreground',
}

const SUCCESS_RATE_COLOR_TOKEN: Record<SuccessRateLevel, string> = {
  excellent: '--forge-chart-success-light',
  good: '--forge-chart-good-light',
  warning: '--forge-chart-warning-light',
  critical: '--forge-chart-critical-light',
  unknown: '--forge-chart-unknown-light',
}

export function getSuccessRateTextClass(
  rate: number | null | undefined
): string {
  return SUCCESS_RATE_TEXT_CLASS[getSuccessRateLevel(rate)]
}

export function getSuccessRateDotClass(
  rate: number | null | undefined
): string {
  return SUCCESS_RATE_DOT_CLASS[getSuccessRateLevel(rate)]
}

export function getSuccessRateColor(rate: number | null | undefined): string {
  const level = getSuccessRateLevel(rate)
  const token = SUCCESS_RATE_COLOR_TOKEN[level]
  const darkToken = token.replace('-light', '-dark')
  const resolvedToken =
    typeof document !== 'undefined' &&
    document.documentElement.classList.contains('dark')
      ? darkToken
      : token
  return resolveForgeColor(resolvedToken)
}

export type PerformanceMetric =
  | 'avg_ttft_ms'
  | 'avg_latency_ms'
  | 'avg_tps'
  | 'success_rate'

export function hasPerformanceCounters(
  row: PerformanceStatistics
): row is PerformanceStatistics & PerformanceCounters {
  return (
    row.request_count !== undefined &&
    row.success_count !== undefined &&
    row.latency_sum_us !== undefined &&
    row.ttft_count !== undefined &&
    row.ttft_sum_us !== undefined &&
    row.output_tokens !== undefined &&
    row.throughput_time_us !== undefined &&
    row.throughput_count !== undefined
  )
}

/** Sum exact integer counters before converting the final ratio for display.
 * Legacy responses have no denominators: retain their unweighted mean only.
 * Never mix native counters and legacy means into a purported weighted value.
 */
export function aggregatePerformanceMetric(
  rows: PerformanceStatistics[],
  metric: PerformanceMetric
): number | null {
  if (rows.length === 0) return null
  if (rows.some((row) => row.success_count !== undefined)) {
    if (!rows.every(hasPerformanceCounters)) return null
    let numerator = 0n
    let denominator = 0n
    for (const row of rows) {
      switch (metric) {
        case 'avg_ttft_ms':
          numerator += BigInt(row.ttft_sum_us)
          denominator += BigInt(row.ttft_count) * 1000n
          break
        case 'avg_latency_ms':
          numerator += BigInt(row.latency_sum_us)
          denominator += BigInt(row.request_count) * 1000n
          break
        case 'avg_tps':
          numerator += BigInt(row.output_tokens) * 1_000_000n
          denominator += BigInt(row.throughput_time_us)
          break
        case 'success_rate':
          numerator += BigInt(row.success_count) * 100n
          denominator += BigInt(row.request_count)
          break
      }
    }
    return denominator > 0n ? Number(numerator) / Number(denominator) : null
  }
  const values = rows.flatMap((row) => {
    const value = row[metric]
    if (value == null || !Number.isFinite(value)) return []
    if (metric === 'success_rate') return [Math.min(100, Math.max(0, value))]
    return value > 0 ? [value] : []
  })
  return values.length
    ? values.reduce((sum, value) => sum + value, 0) / values.length
    : null
}

export function isPerformancePartial(
  data: PerformanceAvailability | undefined
) {
  return (
    data?.availability === 'partial' ||
    (data?.coverage?.missing_samples ?? 0) > 0
  )
}

export function performanceCoverageNotice(
  data: PerformanceAvailability | undefined,
  t: (key: string, options?: Record<string, unknown>) => string
): string | null {
  if (!isPerformancePartial(data)) return null
  if (!data?.coverage) return t('Partial performance coverage')
  return t(
    'Partial coverage: {{sampled}} sampled requests; {{missing}} missing samples. Native relay only; historical imports excluded.',
    {
      sampled: data.coverage.sampled_requests,
      missing: data.coverage.missing_samples,
    }
  )
}

export const RELAY_TIMING_DESCRIPTION =
  'Relay latency includes reservation, upstream execution and backpressure; excludes HTTP authentication and subsequent settlement. Relay throughput is successful output tokens per relay second. TTFT measures the first non-empty streamed text, thinking or tool delta; unavailable for non-streaming requests.'
