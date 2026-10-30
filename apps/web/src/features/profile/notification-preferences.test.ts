import assert from 'node:assert/strict'
import { test } from 'node:test'

import { api } from '@/lib/api'

import {
  notificationPreferencesSchema,
  readNotificationPreferences,
  saveNotificationPreferences,
} from './notification-preferences'

test('notification preferences preserve exact units and reject coercions', () => {
  const value = {
    enabled: true,
    revision: 1,
    low_balance_units: '9007199254740991',
    delivery: 'in_app',
  }
  assert.equal(
    notificationPreferencesSchema.parse(value).low_balance_units,
    value.low_balance_units
  )
  for (const threshold of ['1e4', '-1', '01', '0.5', '9007199254740992']) {
    assert.equal(
      notificationPreferencesSchema.safeParse({
        ...value,
        low_balance_units: threshold,
      }).success,
      false
    )
  }
})

test('notification reads and writes retain the initiating authentication scope and revision', async () => {
  const originalGet = api.get
  const originalPut = api.put
  const scope = { userId: 7, sessionId: 'synthetic-session' }
  const value = {
    enabled: true,
    revision: 3,
    low_balance_units: '500000',
    delivery: 'in_app' as const,
  }
  const calls: unknown[] = []
  api.get = (async (path: string, options: unknown) => {
    calls.push([path, options])
    return { data: { success: true, data: value } }
  }) as typeof api.get
  api.put = (async (path: string, body: unknown, options: unknown) => {
    calls.push([path, body, options])
    return { data: { success: true, data: { ...value, revision: 4 } } }
  }) as typeof api.put
  try {
    assert.deepEqual(await readNotificationPreferences(scope), value)
    assert.equal((await saveNotificationPreferences(scope, value)).revision, 4)
    assert.deepEqual(calls, [
      [
        '/core/v1/notifications/preferences',
        { authScope: scope, signal: undefined },
      ],
      [
        '/core/v1/notifications/preferences',
        { enabled: true, low_balance_units: '500000', revision: 3 },
        { authScope: scope },
      ],
    ])
  } finally {
    api.get = originalGet
    api.put = originalPut
  }
})
