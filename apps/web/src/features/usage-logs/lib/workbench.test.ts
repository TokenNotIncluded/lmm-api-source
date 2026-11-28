/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { usageLogSchema } from '../data/schema'
import { buildSearchParams, validateCommonLogFilters } from './filter'
import { formatDurationMs, getLogTiming } from './timing'
import { buildUsageTrendBuckets } from './trend'
import { buildApiParams } from './utils'

test('one applied native filter produces identical list, statistics and export parameters', () => {
  const filters = {
    startTime: new Date('2026-11-15T01:00:00Z'),
    endTime: new Date('2026-11-15T02:00:00Z'),
    status: 'error' as const,
    model: 'model-a',
    token: 'integration',
    group: 'default',
    username: 'operator',
    channel: '42',
    requestId: 'request-a',
    upstreamRequestId: 'upstream-b',
    minDurationMs: '2.5',
    maxDurationMs: '1500',
    sortOrder: 'asc' as const,
  }
  assert.equal(validateCommonLogFilters(filters, true, false), undefined)
  const searchParams = buildSearchParams(filters, 'common')
  const {
    p: _page,
    page_size: _size,
    ...list
  } = buildApiParams({ page: 2, pageSize: 20, searchParams, isAdmin: true })
  const {
    p: _summaryPage,
    page_size: _summarySize,
    ...summary
  } = buildApiParams({ page: 1, pageSize: 1, searchParams, isAdmin: true })
  assert.deepEqual(list, summary)
  assert.deepEqual(list, {
    model_name: 'model-a',
    token_name: 'integration',
    group: 'default',
    username: 'operator',
    channel: 42,
    request_id: 'request-a',
    upstream_request_id: 'upstream-b',
    status: 'error',
    min_duration_ms: 2.5,
    max_duration_ms: 1500,
    sort_order: 'asc',
    start_timestamp: 1794704400,
    end_timestamp: 1794708000,
  })
  const personal = buildApiParams({
    page: 1,
    pageSize: 20,
    searchParams,
    isAdmin: false,
  })
  assert.equal(personal.username, undefined)
  assert.equal(personal.channel, undefined)
  assert.equal(personal.upstream_request_id, 'upstream-b')
})

test('invalid ranges are kept as a correctable draft instead of issuing a query', () => {
  assert.match(
    validateCommonLogFilters(
      { startTime: new Date(20), endTime: new Date(10) },
      false,
      false
    ) ?? '',
    /time range/
  )
  for (const filters of [
    { minDurationMs: '20', maxDurationMs: '10' },
    { minDurationMs: '-1' },
    { minDurationMs: 'NaN' },
    { maxDurationMs: '86400001' },
  ]) {
    assert.match(
      validateCommonLogFilters(filters, false, false) ?? '',
      /milliseconds/
    )
  }
  assert.match(
    validateCommonLogFilters({ channel: 'invalid' }, true, false) ?? '',
    /Channel ID/
  )
  assert.equal(
    validateCommonLogFilters(
      { minDurationMs: '0', maxDurationMs: '0.5' },
      false,
      false
    ),
    undefined
  )
})

test('native millisecond timing preserves time to first token and whole-request throughput', () => {
  const log = usageLogSchema.parse({
    id: 1,
    user_id: 7,
    created_at: 1,
    type: 2,
    content: '',
    state: 'settled',
    duration_ms: 1500.25,
    first_token_ms: 500.25,
    use_time: 1,
    is_stream: true,
    completion_tokens: 100,
    username: null,
    channel: null,
    upstream_request_id: null,
    price_version: 'price-v3',
    diagnostics_available: true,
  })
  const timing = getLogTiming(log, { frt: 999 })
  assert.equal(timing.durationMs, 1500.25)
  assert.equal(timing.firstTokenMs, 500.25)
  assert.equal(timing.throughput, 100000 / 1500.25)
  assert.equal(formatDurationMs(0.25), '0.25 ms')
  assert.equal(log.price_version, 'price-v3')
  assert.equal(log.channel, null)
})

test('missing native measurements never masquerade as zero latency or non-stream requests', () => {
  const log = usageLogSchema.parse({
    id: 1,
    user_id: 7,
    created_at: 1,
    type: 5,
    content: '',
    state: 'released',
    duration_ms: null,
    first_token_ms: null,
    use_time: null,
    is_stream: null,
  })
  assert.deepEqual(getLogTiming(log, { frt: 250 }), {
    durationMs: null,
    firstTokenMs: null,
    throughput: null,
  })
  assert.equal(log.is_stream, null)
  assert.equal(formatDurationMs(null), '—')
  assert.equal(
    getLogTiming(
      {
        ...log,
        duration_ms: 5,
        first_token_ms: 5,
        completion_tokens: 10,
      },
      null
    ).throughput,
    2000
  )
})

test('historical timing retains existing seconds when no native receipt is present', () => {
  const log = usageLogSchema.parse({
    id: 1,
    user_id: 7,
    created_at: 1,
    type: 2,
    content: '',
    use_time: 2,
    is_stream: true,
    completion_tokens: 100,
  })
  assert.deepEqual(getLogTiming(log, { frt: 500 }), {
    durationMs: 2000,
    firstTokenMs: 500,
    throughput: 50,
  })
})

test('trend buckets cover every selected second exactly once, including short ranges', () => {
  for (const [start, end] of [
    [1704067200000, 1704153600000],
    [1000, 3000],
    [1000, 1000],
  ]) {
    const buckets = buildUsageTrendBuckets(start, end)
    assert.equal(buckets[0]?.start, start)
    assert.equal(buckets.at(-1)?.end, end)
    assert.ok(buckets.length <= 12)
    for (let index = 1; index < buckets.length; index += 1) {
      assert.equal(buckets[index].start, buckets[index - 1].end + 1000)
    }
  }
})
