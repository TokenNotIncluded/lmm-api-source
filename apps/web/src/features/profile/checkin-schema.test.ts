/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import {
  checkinClaimResponseSchema,
  checkinStatusResponseSchema,
} from './checkin-schema'

const record = {
  checkin_date: '2030-01-01',
  quota_awarded: '0',
  created_at: 1893427200,
  journal_id: null,
  origin: 'history',
  policy_revision: null,
  trust_level: null,
}
const exactTotal = (BigInt(Number.MAX_SAFE_INTEGER) + 123456789n).toString()
const status = {
  enabled: true,
  today: '2030-01-01',
  month: '2030-01',
  calendar_offset_seconds: 28800,
  policy_revision: 1,
  checked_in_today: true,
  reward_range: {
    trust_level: 0,
    reward_multiplier: 0,
    base_min_quota: '1',
    base_max_quota: '500000',
    min_quota: '0',
    max_quota: '0',
  },
  today_record: record,
  records: [record],
  total_checkins: 5986,
  total_quota_awarded: exactTotal,
}

test('Native preserves zero, imported day, server calendar and exact totals', () => {
  const response = checkinStatusResponseSchema.parse({
    success: true,
    data: status,
  })
  assert.ok(response.success)
  assert.equal(response.data.kind, 'native')
  assert.equal(response.data.stats.total_quota, exactTotal)
  assert.equal(response.data.stats.records[0]?.quota_awarded, '0')
  const claim = checkinClaimResponseSchema.parse({
    success: true,
    data: { already_claimed: true, record },
  })
  assert.ok(claim.success && claim.data.kind === 'native')
  assert.equal(claim.data.already_claimed, true)
  assert.equal(claim.data.record.journal_id, null)
})

test('old Go numeric response remains supported and normalizes to exact units', () => {
  const response = checkinStatusResponseSchema.parse({
    success: true,
    data: {
      enabled: true,
      min_quota: 0,
      max_quota: 100000,
      trust_level: 1,
      stats: {
        checked_in_today: false,
        total_checkins: 1,
        total_quota: 0,
        checkin_count: 1,
        records: [{ checkin_date: '2030-01-01', quota_awarded: 0 }],
      },
    },
  })
  assert.ok(response.success && response.data.kind === 'legacy')
  assert.equal(response.data.reward_range?.min_quota, '0')
  const claim = checkinClaimResponseSchema.parse({
    success: true,
    data: { quota_awarded: 0 },
  })
  assert.ok(claim.success && claim.data.kind === 'legacy')
  assert.equal(claim.data.quota_awarded, '0')
})

test('invalid dates, mixed periods and invalid monetary representations fail at boundary', () => {
  for (const data of [
    { ...status, today: '2030-02-30' },
    { ...status, checked_in_today: false },
    { ...status, month: '2030-02' },
    { ...status, records: [record, record] },
    { ...status, total_quota_awarded: 9007199254740992 },
    { ...status, total_quota_awarded: '-1' },
    {
      ...status,
      reward_range: { ...status.reward_range, min_quota: '2', max_quota: '1' },
    },
  ]) {
    assert.equal(
      checkinStatusResponseSchema.safeParse({ success: true, data }).success,
      false
    )
  }
})
