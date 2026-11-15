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
import { afterEach, describe, test } from 'node:test'

import { api } from '@/lib/api'

import {
  getProfileUsageWindow,
  performCheckin,
  getCheckinChallenge,
  checkinFailureCode,
  getCheckinStatus,
  enableProfileShare,
  getProfileShareState,
  updateProfileAggregate,
} from './api'

const originalGet = api.get
const originalPost = api.post

afterEach(() => {
  api.get = originalGet
  api.post = originalPost
})

test('linked account settings preserve separate consent and explicit clearing', async () => {
  const requests: unknown[] = []
  api.post = (async (url, body, config) => {
    requests.push({ url, body, config })
    return {
      data: {
        success: true,
        data: {
          enabled: true,
          aggregate_usage_enabled: true,
          model_usage_enabled: false,
        },
      },
    }
  }) as typeof api.post
  const settings = {
    aggregate_usage_enabled: true,
    linked_profiles: [
      { provider: 'cursor' as const, url: 'https://cursor.com/@one' },
    ],
  }
  assert.equal(
    (await updateProfileAggregate(settings)).model_usage_enabled,
    false
  )
  await updateProfileAggregate({
    aggregate_usage_enabled: false,
    linked_profiles: [],
  })
  assert.deepEqual(requests, [
    {
      url: '/api/user/self/profile-share',
      body: settings,
      config: { skipBusinessError: true, skipErrorHandler: true },
    },
    {
      url: '/api/user/self/profile-share',
      body: { aggregate_usage_enabled: false, linked_profiles: [] },
      config: { skipBusinessError: true, skipErrorHandler: true },
    },
  ])
  api.post = (async () => ({
    data: { success: false, message: 'raw private provider detail' },
  })) as typeof api.post
  await assert.rejects(
    updateProfileAggregate(settings),
    /^Error: Unable to save linked profiles$/
  )
})

describe('profile activity API', () => {
  test('requests one bounded self-usage window without global error toasts', async () => {
    let request: { url: string; config: unknown } | null = null
    const rows = [{ created_at: 1_700_000_000, token_used: 42, count: 2 }]

    api.get = (async (url, config) => {
      request = { url, config }
      return { data: { success: true, data: rows } }
    }) as typeof api.get

    assert.deepEqual(
      await getProfileUsageWindow({
        start_timestamp: 1_700_000_000,
        end_timestamp: 1_700_000_100,
      }),
      rows
    )
    assert.deepEqual(request, {
      url: '/api/data/self',
      config: {
        params: {
          start_timestamp: 1_700_000_000,
          end_timestamp: 1_700_000_100,
          default_time: 'day',
        },
        skipBusinessError: true,
        skipErrorHandler: true,
      },
    })
  })
})

describe('profile check-in API', () => {
  test('Native default month is server-owned and explicit month is bound to response', async () => {
    const urls: string[] = []
    api.get = (async (url) => {
      urls.push(String(url))
      return {
        data: {
          success: true,
          data: {
            enabled: false,
            today: '2030-01-01',
            month: '2030-01',
            calendar_offset_seconds: 28800,
            policy_revision: null,
            checked_in_today: false,
            reward_range: null,
            today_record: null,
            records: [],
            total_checkins: 0,
            total_quota_awarded: '0',
          },
        },
      }
    }) as typeof api.get
    const response = await getCheckinStatus()
    assert.ok(response.success && response.data.kind === 'native')
    assert.equal(response.data.month, '2030-01')
    assert.equal(urls[0], '/api/user/checkin')
    await assert.rejects(
      getCheckinStatus('2029-12'),
      /Invalid check-in status month/
    )
    assert.equal(urls[1], '/api/user/checkin?month=2029-12')
  })

  test('sends verification only in JSON and binds the request to its original account', async () => {
    let request: {
      url: string
      body: unknown
      config: unknown
    } | null = null

    api.post = (async (url, body, config) => {
      request = { url, body, config }
      return {
        data: {
          success: false,
          error: { code: 'CHECKIN_VERIFICATION_FAILED' },
        },
      }
    }) as typeof api.post

    const controller = new AbortController()
    const authScope = { userId: 12, sessionId: 'original-session' }
    const body = {
      operation_key: 'a-valid-operation-key',
      challenge_id: 'server-challenge',
      turnstile_token: 'turnstile-token',
      passkey_assertion: { id: 'credential-id' },
    }
    const result = await performCheckin(body, {
      signal: controller.signal,
      authScope,
    })
    assert.equal(checkinFailureCode(result), 'CHECKIN_VERIFICATION_FAILED')

    assert.deepEqual(request, {
      url: '/api/user/checkin',
      body,
      config: {
        skipBusinessError: true,
        skipErrorHandler: true,
        authScope,
        signal: controller.signal,
        timeout: 20_000,
      },
    })
  })

  test('requests a challenge with only the operation key and preserves structured HTTP errors', async () => {
    let request: unknown
    api.post = (async (url, body) => {
      request = { url, body }
      return {
        data: { success: false, error: { code: 'CHECKIN_PASSKEY_REQUIRED' } },
      }
    }) as typeof api.post
    await getCheckinChallenge('a-valid-operation-key')
    assert.deepEqual(request, {
      url: '/api/user/checkin/challenges',
      body: { operation_key: 'a-valid-operation-key' },
    })
    assert.equal(
      checkinFailureCode({
        isAxiosError: true,
        response: {
          data: {
            success: false,
            error: { code: 'CHECKIN_VERIFICATION_REQUIRED' },
          },
        },
      }),
      'CHECKIN_VERIFICATION_REQUIRED'
    )
    assert.equal(
      checkinFailureCode(new Error('private server detail')),
      undefined
    )
  })
})

test('model-share consent sends explicit booleans and preserves body-less enable', async () => {
  const bodies: unknown[] = []
  api.post = (async (_url, body) => {
    bodies.push(body)
    const consent = body as Record<string, unknown> | undefined
    return {
      data: {
        success: true,
        data: {
          enabled: true,
          model_usage_enabled: consent?.model_usage_enabled === true,
        },
      },
    }
  }) as typeof api.post
  await enableProfileShare()
  assert.equal((await enableProfileShare(true)).model_usage_enabled, true)
  assert.equal((await enableProfileShare(false)).model_usage_enabled, false)
  assert.deepEqual(bodies, [
    undefined,
    { model_usage_enabled: true },
    { model_usage_enabled: false },
  ])
})

test('profile share reads pass the query AbortSignal to axios', async () => {
  const controller = new AbortController()
  let observedSignal: AbortSignal | undefined
  api.get = (async (_url, config) => {
    observedSignal = config?.signal as AbortSignal | undefined
    return { data: { success: true, data: { enabled: true } } }
  }) as typeof api.get
  await getProfileShareState(controller.signal)
  assert.equal(observedSignal, controller.signal)
})
