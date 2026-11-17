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
import assert from 'node:assert/strict'
import { describe, test } from 'node:test'

import { api } from '@/lib/api'

import {
  getHeroSmsPreviewCustomerPrice,
  getHeroSmsSettings,
  serializeHeroSmsSettingsUpdate,
  testHeroSmsConnection,
  toHeroSmsSettingsFormValues,
  updateHeroSmsSettings,
} from './hero-sms-api'

describe('hero sms settings api helpers', () => {
  test('writes only activation settings with the displayed revision', () => {
    assert.deepEqual(
      serializeHeroSmsSettingsUpdate({
        revision: '9007199254740993',
        enabled: true,
        emailEnabled: true,
        smsEnabled: false,
        priceMultiplier: 1,
      }),
      {
        expected_revision: '9007199254740993',
        enabled: true,
        email_enabled: true,
        sms_enabled: false,
        price_multiplier: '1',
      }
    )

    assert.deepEqual(
      serializeHeroSmsSettingsUpdate({
        revision: '4',
        enabled: false,
        emailEnabled: false,
        smsEnabled: true,
        priceMultiplier: 12,
      }),
      {
        expected_revision: '4',
        enabled: false,
        email_enabled: false,
        sms_enabled: true,
        price_multiplier: '12',
      }
    )
  })

  test('builds form defaults and preview values', () => {
    assert.deepEqual(
      toHeroSmsSettingsFormValues({
        revision: '4',
        enabled: true,
        email_enabled: true,
        sms_enabled: true,
        api_key_configured: true,
        credential_managed_by_file: true,
        pending_work: false,
        currency: 'USD',
        currency_code: 840,
        price_multiplier: '12.5',
      }),
      {
        revision: '4',
        enabled: true,
        emailEnabled: true,
        smsEnabled: true,
        priceMultiplier: 12.5,
      }
    )

    assert.equal(getHeroSmsPreviewCustomerPrice(1), 1)
    assert.equal(getHeroSmsPreviewCustomerPrice(12.5), 12.5)
  })
})

test('native settings save the browser revision and test only the host credential', async () => {
  const prior = api.defaults.adapter
  const settings = {
    revision: '9007199254740993',
    enabled: true,
    email_enabled: true,
    sms_enabled: true,
    api_key_configured: true,
    credential_managed_by_file: true,
    pending_work: false,
    currency: 'USD',
    currency_code: 840,
    price_multiplier: '12.5',
  }
  const writes: { url?: string; body: unknown }[] = []
  api.defaults.adapter = async (config) => {
    if (config.method !== 'get') {
      assert.equal(config.skipAuthRefresh, true)
      writes.push({ url: config.url, body: JSON.parse(config.data) })
    }
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: {
        success: true,
        data: config.method === 'post' ? { ok: true } : settings,
      },
    }
  }
  try {
    const loaded = await getHeroSmsSettings()
    assert.equal(loaded.credential_managed_by_file, true)
    const draft = {
      ...toHeroSmsSettingsFormValues(loaded),
      apiKey: 'synthetic-ignored-key',
    }
    await updateHeroSmsSettings(draft)
    await testHeroSmsConnection()
    assert.deepEqual(writes, [
      {
        url: '/api/option/hero-sms',
        body: {
          expected_revision: settings.revision,
          enabled: true,
          email_enabled: true,
          sms_enabled: true,
          price_multiplier: '12.5',
        },
      },
      { url: '/api/option/hero-sms/test', body: {} },
    ])
  } finally {
    api.defaults.adapter = prior
  }
})

test('a negative connection result is not reported as a successful test', async () => {
  const prior = api.defaults.adapter
  api.defaults.adapter = async (config) => ({
    config,
    headers: {},
    status: 200,
    statusText: 'OK',
    data: { success: true, data: { ok: false } },
  })
  try {
    await assert.rejects(testHeroSmsConnection(), /connection test failed/)
  } finally {
    api.defaults.adapter = prior
  }
})
