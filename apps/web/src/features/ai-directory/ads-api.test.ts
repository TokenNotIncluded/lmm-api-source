/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { api } from '@/lib/api'

import { prepareDirectoryAd } from './ads-api'

const draft = {
  name: 'Placement',
  url: 'https://example.test',
  summary: '',
  description: '',
  request_id: 'ad-request-1234567890',
  bid_cents: 100,
}
const quote = {
  pricing_schema_version: 2,
  quote_contract_version: 1,
  currency: 'USD',
  bid_cents: 100,
  quota: 3_500_000,
  duration_days: 30,
  min_bid_cents: 100,
  max_bid_cents: 1_000_000,
  quote_id: 'a'.repeat(64),
  request_id: draft.request_id,
  content_sha256: 'b'.repeat(64),
  expires_at: Math.floor(Date.now() / 1000) + 300,
  confirmation_path: `/core/v1/directory-ads/confirm/${'a'.repeat(64)}`,
}

test('prepares the complete draft and retains the exact server charge and confirmation path', async () => {
  const original = api.post
  api.post = (async (url: string, input: unknown) => {
    assert.equal(url, '/api/ai-directory/ads/prepare')
    assert.deepEqual(input, draft)
    return { data: { success: true, data: quote } }
  }) as typeof api.post
  try {
    assert.deepEqual(await prepareDirectoryAd(draft), quote)
  } finally {
    api.post = original
  }
})

test('rejects stale, mismatched and unsafe prepared quotes before opening confirmation', async () => {
  const original = api.post
  try {
    for (const invalid of [
      { ...quote, pricing_schema_version: undefined },
      { ...quote, quote_contract_version: 0 },
      { ...quote, currency: 'CNY' },
      { ...quote, quota: 0 },
      { ...quote, quota: 3.5 },
      { ...quote, quota: Number.MAX_SAFE_INTEGER + 1 },
      { ...quote, request_id: 'another-request' },
      { ...quote, bid_cents: 200 },
      { ...quote, expires_at: 1 },
      { ...quote, content_sha256: '' },
      { ...quote, confirmation_path: 'https://external.example/confirm' },
      { ...quote, confirmation_path: '//external.example/confirm' },
      {
        ...quote,
        confirmation_path: `/core/v1/directory-ads/confirm/${'c'.repeat(64)}`,
      },
    ]) {
      api.post = (async () => ({
        data: { success: true, data: invalid },
      })) as typeof api.post
      await assert.rejects(
        prepareDirectoryAd(draft),
        /Unable to get a price quote/
      )
    }
  } finally {
    api.post = original
  }
})
