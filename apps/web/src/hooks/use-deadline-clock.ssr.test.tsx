/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { createInstance } from 'i18next'
import { renderToStaticMarkup } from 'react-dom/server'
import { I18nextProvider, initReactI18next } from 'react-i18next'

import { StoreBadges } from '@/features/store/shared'
import type { StoreProduct } from '@/features/store/types'

import { useCurrentMinute, useLocalDay } from './use-deadline-clock'

const i18n = createInstance()
await i18n.use(initReactI18next).init({
  lng: 'en',
  resources: { en: { translation: {} } },
})

for (const state of ['expired', 'live', 'zero'] as const) {
  test(`static promotion badge uses real server time for ${state} deadline`, () => {
    const deadline =
      state === 'zero' ? 0 : Date.now() / 1000 + (state === 'live' ? 60 : -60)
    const product = {
      status: 'published',
      trading_paused: false,
      unlimited_supply: true,
      sale_limit: null,
      promotion_expires_at: deadline,
    } as StoreProduct
    const markup = renderToStaticMarkup(
      <I18nextProvider i18n={i18n}>
        <StoreBadges product={product} />
      </I18nextProvider>
    )
    assert.equal(
      markup.includes('aria-label="Promoted product"'),
      state === 'live'
    )
  })
}

test('static calendar and minute use the current local day and minute, not an epoch fallback', () => {
  function Calendar() {
    const day = useLocalDay()
    const minute = useCurrentMinute()
    return <output data-day={day} data-minute={minute} />
  }
  const before = Date.now()
  const markup = renderToStaticMarkup(<Calendar />)
  const after = Date.now()
  const day = Number(/data-day="([^"]+)"/.exec(markup)?.[1])
  const minute = Number(/data-minute="([^"]+)"/.exec(markup)?.[1])
  assert.ok(
    day >= new Date(before).setHours(0, 0, 0, 0) &&
      day <= new Date(after).setHours(0, 0, 0, 0)
  )
  assert.ok(
    minute >= Math.floor(before / 60_000) * 60_000 &&
      minute <= Math.floor(after / 60_000) * 60_000
  )
  assert.ok(day > 0 && minute > 0)
})
