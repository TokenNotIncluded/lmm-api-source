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
import { test } from 'node:test'

import { api } from '@/lib/api'

import { getHeroSmsSettings, testHeroSmsConnection } from './hero-sms-api'

test('the status projection reads capabilities independently of the legacy purchase policy', async () => {
  const prior = api.defaults.adapter
  const status = {
    api_key_configured: true,
    credential_managed_by_file: true,
    pending_work: false,
    purchases_available: false,
  }
  api.defaults.adapter = async (config) => {
    assert.equal(config.method, 'get')
    assert.equal(config.url, '/api/option/hero-sms')
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: {
        success: true,
        data: { ...status, enabled: true, price_multiplier: '12' },
      },
    }
  }
  try {
    assert.deepEqual(await getHeroSmsSettings(), status)
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
