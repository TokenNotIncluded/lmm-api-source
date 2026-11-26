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

import { createInstance } from 'i18next'

import type { PerformanceAvailability, PerformanceStatistics } from '../types'
import {
  aggregatePerformanceMetric,
  formatLatency,
  formatThroughput,
  formatUptimePct,
  getSuccessRateLevel,
  hasScopedNativeSamples,
  isPerformancePartial,
  performanceCoverageNotice,
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

test('missing native counters never fall back to averaging old aggregate rates', () => {
  const old = {
    avg_latency_ms: 10,
    avg_tps: 20,
    avg_ttft_ms: 5,
    success_rate: 80,
  }
  for (const metric of [
    'avg_latency_ms',
    'avg_tps',
    'avg_ttft_ms',
    'success_rate',
  ] as const) {
    assert.equal(aggregatePerformanceMetric([old], metric), null)
    assert.equal(aggregatePerformanceMetric([aggregate, old], metric), null)
  }
})

const nativeWindow = {
  native_shared_relay: true,
  historical_imports: false,
  unreserved_http_requests: false,
  counts: 'accepted_logical_requests_with_samples',
  sampled_requests: 9,
  missing_samples: 0,
  window_start: 100,
  window_end_exclusive: 200,
}

const english = createInstance()
await english.init({ lng: 'en', resources: { en: { translation: {} } } })

function notice(data: PerformanceAvailability) {
  const message = performanceCoverageNotice(data, english.t)
  assert.ok(message !== null)
  return message
}

test('a fully sampled native window remains partial in scope', () => {
  for (const sampled of [9, 0]) {
    const data = {
      availability: 'partial',
      coverage: { ...nativeWindow, sampled_requests: sampled },
    } as const
    assert.equal(isPerformancePartial(data), true)
    assert.equal(hasScopedNativeSamples(data), true)
    assert.equal(
      notice(data),
      `Native relay samples in this window: ${sampled}. No missing samples. Historical imports excluded.`
    )
  }
})

test('missing samples, other scopes and incomplete evidence keep the warning', () => {
  const gaps = {
    availability: 'partial',
    coverage: { ...nativeWindow, missing_samples: 2 },
  } as const
  assert.equal(hasScopedNativeSamples(gaps), false)
  assert.match(notice(gaps), /2 missing samples/)

  for (const change of [
    { native_shared_relay: false },
    { historical_imports: true },
    { counts: 'provider_partial' },
    { counts: undefined },
    { unreserved_http_requests: undefined },
    { sampled_requests: -1 },
  ]) {
    const data = {
      availability: 'partial',
      coverage: { ...nativeWindow, ...change },
    } as const
    assert.equal(hasScopedNativeSamples(data), false)
    assert.match(notice(data), /Partial/)
  }
  assert.equal(
    notice({ availability: 'partial' }),
    'Partial performance coverage'
  )
  const other = {
    ...gaps,
    coverage: { ...gaps.coverage, native_shared_relay: false },
  }
  assert.doesNotMatch(notice(other), /Native relay only/)
})
