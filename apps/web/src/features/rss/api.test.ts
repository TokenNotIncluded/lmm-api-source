/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import { api } from '@/lib/api'

import {
  getRSSFeeds,
  getRSSSettings,
  saveRSSSettings,
  parseRSSFeeds,
  safeRSSFeedUrl,
} from './api'

const adapter = api.defaults.adapter
afterEach(() => {
  api.defaults.adapter = adapter
})

test('RSS feed configuration accepts safe HTTP feeds', () => {
  const feeds = parseRSSFeeds(
    JSON.stringify([
      {
        id: 'release-notes',
        name: 'Release notes',
        url: 'https://example.com/feed.xml',
        enabled: true,
      },
    ])
  )
  assert.equal(feeds?.length, 1)
  assert.equal(feeds?.[0]?.name, 'Release notes')
})

test('RSS feed configuration rejects duplicate and credentialed URLs', () => {
  assert.equal(
    parseRSSFeeds(
      JSON.stringify([
        {
          id: 'a',
          name: 'A',
          url: 'https://example.com/feed.xml',
          enabled: true,
        },
        {
          id: 'b',
          name: 'B',
          url: 'https://example.com/feed.xml',
          enabled: true,
        },
      ])
    ),
    null
  )
  assert.equal(safeRSSFeedUrl('https://user:pass@example.com/feed.xml'), null)
  assert.equal(safeRSSFeedUrl('file:///tmp/feed.xml'), null)
  assert.equal(
    safeRSSFeedUrl('https://example.com/feed.xml#section'),
    'https://example.com/feed.xml'
  )
})

test('RSS settings keep revision and explicit empty feeds through the API', async () => {
  const calls: string[] = []
  api.defaults.adapter = async (config) => {
    calls.push(`${config.method} ${config.url}`)
    if (config.method === 'put') {
      assert.deepEqual(JSON.parse(config.data), { revision: 4, feeds: [] })
      assert.equal(config.skipAuthRefresh, true)
    }
    return {
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: { success: true, data: { revision: 5, feeds: [] } },
    }
  }
  const options = { authMode: 'anonymous' as const, skipAuthRefresh: true }
  assert.deepEqual(await getRSSSettings(options), { revision: 5, feeds: [] })
  assert.deepEqual(await saveRSSSettings({ revision: 4, feeds: [] }, options), {
    revision: 5,
    feeds: [],
  })
  assert.deepEqual(calls, ['get /api/rss/settings', 'put /api/rss/settings'])
})

test('RSS malformed and unavailable payloads are never successful empty data', async () => {
  const options = {
    authMode: 'anonymous' as const,
    skipAuthRefresh: true,
    skipBusinessError: true,
    disableDuplicate: true,
  }
  for (const data of [
    { success: true, data: { feeds: null } },
    { success: true, data: { revision: -1, feeds: [] } },
    { success: false, data: { revision: 0, feeds: [] } },
  ]) {
    api.defaults.adapter = async (config) => ({
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data,
    })
    await assert.rejects(() => getRSSSettings(options))
  }
  api.defaults.adapter = async (config) => ({
    config,
    status: 200,
    statusText: 'OK',
    headers: {},
    data: { success: true, data: { feeds: null } },
  })
  await assert.rejects(() => getRSSFeeds(options))
})
