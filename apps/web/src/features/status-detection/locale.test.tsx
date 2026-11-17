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
// @ts-expect-error Bun's test module is available only in the test runtime.
import { mock } from 'bun:test'
import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import { createInstance } from 'i18next'
import type { ReactNode } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { I18nextProvider } from 'react-i18next'

import type { StatusGroup } from './types'

mock.module('@tanstack/react-router', () => ({
  getRouteApi: () => ({
    useSearch: () => ({ hours: 24 }),
    useNavigate: () => () => {},
  }),
}))
mock.module('@/features/forge/forge-public-shell', () => ({
  ForgePublicShell: ({ children }: { children: ReactNode }) => <>{children}</>,
}))
mock.module('@/features/pricing/hooks/use-pricing-data', () => ({
  usePricingData: () => ({ models: [] }),
}))
let latestTimestamp: number | null = 1_790_000_000
let groups: StatusGroup[] = []
let coverage:
  | {
      availability: 'partial'
      coverage: { sampled_requests: number; missing_samples: number }
    }
  | undefined
let error: Error | null = null
let isLoading = false
mock.module('./hooks/use-status-detection', () => ({
  useStatusDetection: () => ({
    groups,
    coverage,
    availableModels: [],
    availableGroups: [],
    hours: 24,
    latestTimestamp,
    modelsWithData: groups.length,
    modelCount: groups.length,
    failedModelCount: 0,
    isLoading,
    isFetching: false,
    error,
    refresh: async () => {},
  }),
}))
const { StatusDetection } = await import('./index')

afterEach(() => {
  latestTimestamp = 1_790_000_000
  groups = []
  coverage = undefined
  error = null
  isLoading = false
})

async function statusMarkup() {
  const i18n = createInstance()
  await i18n.init({
    lng: 'en',
    resources: {},
    interpolation: { escapeValue: false },
  })
  return renderToStaticMarkup(
    <I18nextProvider i18n={i18n}>
      <StatusDetection />
    </I18nextProvider>
  )
}

test('compact status cards retain partial coverage, stale samples and unknown readings', async () => {
  groups = [
    {
      group: 'sampled-group',
      avgTtftMs: 120,
      avgLatencyMs: 400,
      avgTps: 25,
      successRate: 100,
      successTrend: [100],
      ttftTrend: [120],
      modelCount: 1,
    },
  ]
  coverage = {
    availability: 'partial',
    coverage: { sampled_requests: 8, missing_samples: 2 },
  }
  latestTimestamp = Date.now() - 7 * 60 * 60 * 1000
  const partial = await statusMarkup()
  assert.match(
    partial,
    /Partial coverage: 8 sampled requests; 2 missing samples/
  )
  assert.match(partial, /Partial performance coverage/)
  assert.match(partial, /more than 6 hours old/)
  assert.match(partial, /Data coverage: 1 of 1 models reported/)

  coverage = undefined
  latestTimestamp = null
  groups = [
    {
      ...groups[0],
      successRate: Number.NaN,
      avgTtftMs: Number.NaN,
      avgLatencyMs: Number.NaN,
      avgTps: Number.NaN,
      successTrend: [],
      ttftTrend: [],
    },
  ]
  const unknown = await statusMarkup()
  assert.match(unknown, /No data/)
  assert.match(unknown, /Performance window: last 24 hours/)
  assert.doesNotMatch(unknown, /100%|0 ms|more than 6 hours old/)
})

test('loading and failed requests do not present zero coverage as measured data', async () => {
  isLoading = true
  assert.doesNotMatch(await statusMarkup(), /Data coverage: 0 of 0/)
  isLoading = false
  error = new Error('synthetic request failure')
  const failed = await statusMarkup()
  assert.match(failed, /Unable to load status data/)
  assert.match(failed, /Retry/)
  assert.doesNotMatch(
    failed,
    /Data coverage: 0 of 0|No status data is available yet/
  )
})

test('status freshness renders across interface language changes and timestamp units', async () => {
  const i18n = createInstance()
  await i18n.init({
    lng: 'en',
    fallbackLng: false,
    resources: {},
    interpolation: { escapeValue: false },
  })
  for (const [language, expectedLocale] of [
    ['en', 'en'],
    ['zh-CN', 'zh-CN'],
    ['zh-TW', 'zh-TW'],
    ['ja', 'ja'],
    ['invalid_locale', undefined],
    ['en', 'en'],
  ] as const) {
    await i18n.changeLanguage(language)
    for (const timestamp of [1_790_000_000, 1_790_000_000_000]) {
      latestTimestamp = timestamp
      const markup = renderToStaticMarkup(
        <I18nextProvider i18n={i18n}>
          <StatusDetection />
        </I18nextProvider>
      )
      const expected = new Date(1_790_000_000_000).toLocaleString(
        expectedLocale
      )
      assert.ok(markup.includes(`Latest data: ${expected}`), language)
    }
  }
  for (const timestamp of [null, Number.NaN]) {
    latestTimestamp = timestamp
    const markup = renderToStaticMarkup(
      <I18nextProvider i18n={i18n}>
        <StatusDetection />
      </I18nextProvider>
    )
    assert.ok(markup.includes('Performance window: last 24 hours'))
  }
})
