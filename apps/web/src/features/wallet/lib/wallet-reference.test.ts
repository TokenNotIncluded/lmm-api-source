/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { mapStatusDataToConfig } from '@/hooks/use-system-config'
import type { CurrencyConfig } from '@/stores/system-config-store'

import { walletReferenceValue } from './wallet-reference'

const config = mapStatusDataToConfig({
  currency_unit: 'credit',
  credit_unit_schema_version: 2,
  credits_per_usd: 500000,
  credits_per_usd_exact: '500000',
  ledger_quota_per_usd: 500000,
  ledger_quota_per_usd_exact: '500000',
  quota_unit: 'LEDGER_QUOTA',
  cny_per_usd: '6.700304',
}).currency as CurrencyConfig

test('references use exact declared rates and cent rounding without legacy display flags', () => {
  assert.equal(
    walletReferenceValue(8065639, 'CNY', config, 'en-US'),
    '108.08 CNY'
  )
  assert.equal(
    walletReferenceValue(8065639, 'USD', config, 'en-US'),
    '16.13 USD'
  )
  assert.equal(
    walletReferenceValue(Number.MAX_SAFE_INTEGER, 'USD', config, 'en-US'),
    '18,014,398,509.48 USD'
  )
  assert.equal(
    walletReferenceValue(-70, 'USD', config, 'en-US'),
    '-0.00014 USD'
  )
})

test('unknown amounts and unavailable or mismatched calibration never invent a reference value', () => {
  assert.equal(walletReferenceValue(null, 'CNY', config, 'en-US'), null)
  assert.equal(
    walletReferenceValue(Number.MAX_SAFE_INTEGER + 1, 'USD', config, 'en-US'),
    null
  )
  for (const overrides of [
    { creditUnitSchemaVersion: undefined },
    { quotaUnit: 'CREDIT' },
    { ledgerQuotaPerUsdExact: '1000000' },
    { creditsPerUsd: 0 },
  ]) {
    assert.equal(
      walletReferenceValue(500000, 'USD', { ...config, ...overrides }, 'en-US'),
      null
    )
  }
  const noFX = { ...config, cnyPerUsd: 0, cnyPerUsdExact: '' }
  assert.equal(walletReferenceValue(500000, 'CNY', noFX, 'en-US'), null)
  assert.equal(walletReferenceValue(500000, 'USD', noFX, 'en-US'), '1 USD')
})
