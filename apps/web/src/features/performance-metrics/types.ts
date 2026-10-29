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
/** Native counters are optional only for compatibility with legacy responses. */
export type PerformanceCounters = {
  request_count: number
  success_count: number
  latency_sum_us: string
  ttft_count: number
  ttft_sum_us: string
  output_tokens: string
  throughput_time_us: string
  throughput_count: number
}

export type PerformanceStatistics = Partial<PerformanceCounters> & {
  avg_ttft_ms?: number | null
  avg_latency_ms: number | null
  success_rate: number | null
  avg_tps: number | null
}

export type PerformanceCoverage = {
  native_shared_relay: boolean
  historical_imports: boolean
  missing_samples: number
  sampled_requests: number
  window_start: number
  window_end_exclusive: number
  bucket_seconds?: 60 | 300 | 3600
}

export type PerformanceAvailability = {
  availability?: 'partial' | 'available' | 'unavailable'
  coverage?: PerformanceCoverage
}

export type PerformanceSeriesPoint = PerformanceStatistics & { ts: number }

export type PerformanceGroup = PerformanceStatistics & {
  group: string
  series: PerformanceSeriesPoint[]
}

export type PerformanceMetricsData = PerformanceAvailability & {
  success: boolean
  message?: string
  data: {
    model_name: string
    series_schema?: string
    groups: PerformanceGroup[]
  }
}

export type PerfModelSummary = PerformanceStatistics & {
  model_name: string
  recent_success_rates?: (number | null)[]
}

export type PerfSummaryAllData = PerformanceAvailability & {
  success: boolean
  message?: string
  data: {
    models: PerfModelSummary[]
    aggregate?: PerformanceStatistics
  }
}
