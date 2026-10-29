/*
Copyright (C) 2026 LIghtJUNction

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
*/
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { createInstance } from 'i18next'
import { renderToStaticMarkup } from 'react-dom/server'
import { I18nextProvider, initReactI18next } from 'react-i18next'

import type { PerfSummaryAllData } from '@/features/performance-metrics/types'

import { PerformanceHealthPanel } from './performance-health-panel'

const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
function render(data: PerfSummaryAllData) {
  const client = new QueryClient()
  client.setQueryData(['perf-metrics-summary', 24], data)
  try {
    return renderToStaticMarkup(
      <I18nextProvider i18n={i18n}>
        <QueryClientProvider client={client}>
          <PerformanceHealthPanel />
        </QueryClientProvider>
      </I18nextProvider>
    )
  } finally {
    client.clear()
  }
}

const data: PerfSummaryAllData = {
  success: true,
  availability: 'partial',
  coverage: {
    native_shared_relay: true,
    historical_imports: false,
    missing_samples: 1,
    sampled_requests: 3,
    window_start: 0,
    window_end_exclusive: 60,
  },
  data: {
    models: [
      {
        model_name: 'model',
        avg_latency_ms: 999,
        avg_tps: 999,
        success_rate: 100,
      },
    ],
    aggregate: {
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
    },
  },
}

test('panel prefers the real aggregate over model means and exposes partial coverage', () => {
  const html = render(data)
  assert.match(html, /66.67%/)
  assert.match(html, /183ms/)
  assert.match(html, /113.9 t\/s/)
  assert.match(html, /3 sampled requests; 1 missing samples/)
  assert.match(html, /Relay throughput/)
  assert.doesNotMatch(html, /forge-chart-success/)
})

test('partial coverage is visible even when no samples exist', () => {
  const html = render({ ...data, data: { models: [] } })
  assert.match(html, /1 missing samples/)
  assert.match(html, /No data available/)
})

test('null aggregate metrics stay unknown instead of falling back to model success', () => {
  const html = render({
    ...data,
    data: {
      ...data.data,
      aggregate: { avg_latency_ms: null, avg_tps: null, success_rate: null },
    },
  })
  assert.equal(html.match(/>—</g)?.length, 3)
})
