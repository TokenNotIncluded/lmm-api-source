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
import { afterEach, describe, test } from 'node:test'

import { api } from '@/lib/api'

import {
  getServiceHealth,
  getStatusDetectionMetrics,
  getStatusDetectionSummary,
} from './api'

const originalGet = api.get

afterEach(() => {
  api.get = originalGet
})

describe('status detection API', () => {
  test('checks public readiness without renewing or sending account credentials', async () => {
    api.get = (async (url, config) => {
      assert.equal(url, '/health/ready')
      assert.equal(config?.authMode, 'anonymous')
      assert.equal(config?.skipAuthRefresh, true)
      assert.equal(config?.validateStatus?.(200), true)
      assert.equal(config?.validateStatus?.(503), true)
      assert.equal(config?.validateStatus?.(404), false)
      return {
        status: 200,
        data: {
          service: 'lmm-core',
          status: 'ready',
          database: true,
          capabilities: { identity: true, models: false },
          readiness_scope: 'configured_components',
          upstream_health_checked: false,
        },
      }
    }) as typeof api.get

    const health = await getServiceHealth()
    assert.equal(health.status, 'ready')
    assert.equal(health.capabilities.models, false)
  })

  test('retains a real unavailable response and rejects a static HTML fallback', async () => {
    api.get = (async () => ({
      status: 503,
      data: {
        service: 'lmm-core',
        status: 'unavailable',
        database: false,
        capabilities: { identity: true, models: true },
        readiness_scope: 'configured_components',
        upstream_health_checked: false,
      },
    })) as typeof api.get
    assert.equal((await getServiceHealth()).status, 'unavailable')

    api.get = (async () => ({
      status: 200,
      data: '<html>SPA fallback</html>',
    })) as typeof api.get
    await assert.rejects(getServiceHealth)
  })

  test('rejects an unsuccessful summary response', async () => {
    api.get = (async () => ({
      data: {
        success: false,
        message: 'Performance service unavailable',
        data: { models: [] },
      },
    })) as typeof api.get

    await assert.rejects(
      () => getStatusDetectionSummary(),
      /Performance service unavailable/
    )
  })

  test('counts unsuccessful detail responses without hiding successful data', async () => {
    api.get = (async (
      _url: string,
      config?: { params?: { model?: string } }
    ) => {
      const modelName = config?.params?.model
      if (modelName === 'failed-model') {
        return {
          data: {
            success: false,
            message: 'Model metrics unavailable',
            data: { model_name: modelName, groups: [] },
          },
        }
      }
      return {
        data: {
          success: true,
          data: {
            model_name: modelName,
            groups: [
              {
                group: 'default',
                avg_ttft_ms: 100,
                avg_latency_ms: 200,
                success_rate: 99,
                avg_tps: 20,
                series: [],
              },
            ],
          },
        },
      }
    }) as typeof api.get

    const result = await getStatusDetectionMetrics([
      'working-model',
      'failed-model',
    ])

    assert.deepEqual(result.failedModels, ['failed-model'])
    assert.deepEqual(
      result.entries.map((entry) => entry.modelName),
      ['working-model']
    )
  })

  test('rejects when every detail response fails', async () => {
    api.get = (async () => ({
      data: {
        success: false,
        message: 'Performance service unavailable',
        data: { model_name: '', groups: [] },
      },
    })) as typeof api.get

    await assert.rejects(
      () => getStatusDetectionMetrics(['alpha', 'beta']),
      /Unable to load performance details/
    )
  })
})
