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
import assert from 'node:assert/strict'
import { test } from 'node:test'

import type { PerformanceStatistics } from '../types'
import {
  aggregatePerformanceMetric,
  formatLatency,
  formatThroughput,
  formatUptimePct,
  getSuccessRateLevel,
} from './format'

// Field values from the real local Native + PG relay receipt (pm1030a4).
const aggregate: PerformanceStatistics = {
  avg_latency_ms: 183.377,
  avg_tps: 113.85696149926846,
  avg_ttft_ms: 135.1295,
  success_rate: 66.66666666666666,
  latency_sum_us: '550131',
  output_tokens: '40',
  request_count: 3,
  success_count: 2,
  throughput_count: 2,
  throughput_time_us: '351318',
  ttft_count: 2,
  ttft_sum_us: '270259',
}

test('real native receipt retains units and denominators', () => {
  assert.equal(
    aggregatePerformanceMetric([aggregate], 'avg_latency_ms'),
    183.377
  )
  assert.equal(aggregatePerformanceMetric([aggregate], 'avg_ttft_ms'), 135.1295)
  assert.equal(
    aggregatePerformanceMetric([aggregate], 'avg_tps'),
    40 / 0.351318
  )
  assert.ok(
    Math.abs(
      (aggregatePerformanceMetric([aggregate], 'success_rate') ?? 0) - 200 / 3
    ) < 1e-12
  )
})

test('null is missing while measured zero remains a valid display', () => {
  assert.equal(formatLatency(null), '—')
  assert.equal(formatThroughput(null), '—')
  assert.equal(formatUptimePct(null), '—')
  assert.equal(getSuccessRateLevel(null), 'unknown')
  assert.equal(formatThroughput(0), '0.00 t/s')
  assert.equal(formatUptimePct(0), '0.00%')
})

test('non-streaming rows do not dilute streaming TTFT', () => {
  const nonstream = {
    ...aggregate,
    ttft_count: 0,
    ttft_sum_us: '0',
    avg_ttft_ms: null,
  }
  assert.equal(
    aggregatePerformanceMetric([aggregate, nonstream], 'avg_ttft_ms'),
    135.1295
  )
})
