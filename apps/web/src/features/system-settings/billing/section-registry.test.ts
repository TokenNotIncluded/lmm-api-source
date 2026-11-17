/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { createInstance } from 'i18next'

import {
  BILLING_SECTION_IDS,
  getBillingSectionNavItems,
} from './section-registry'

test('billing navigation removes account tiers and keeps native pricing and payment settings', async () => {
  const i18n = createInstance()
  await i18n.init({ lng: 'en', resources: { en: { translation: {} } } })
  assert.equal(
    BILLING_SECTION_IDS.some((id) => String(id) === 'levels-benefits'),
    false
  )
  assert.ok(BILLING_SECTION_IDS.includes('model-pricing'))
  assert.ok(BILLING_SECTION_IDS.includes('group-pricing'))
  assert.ok(BILLING_SECTION_IDS.includes('router-pricing'))
  assert.ok(BILLING_SECTION_IDS.includes('payment'))
  assert.equal(
    getBillingSectionNavItems(i18n.t).some((item) =>
      item.url.includes('levels-benefits')
    ),
    false
  )
})
