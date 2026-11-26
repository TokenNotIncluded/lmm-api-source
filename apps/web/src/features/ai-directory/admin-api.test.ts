/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import { api } from '@/lib/api'

import { getDirectorySettings, saveDirectorySettings } from './admin-api'
import { DEFAULT_AI_DIRECTORY_LINKS } from './api'

const originalFetch = globalThis.fetch
const originalPut = api.put
afterEach(() => {
  globalThis.fetch = originalFetch
  api.put = originalPut
})

const link = { ...DEFAULT_AI_DIRECTORY_LINKS[0], enabled: false }
function read(links: unknown) {
  globalThis.fetch = async (url, options) => {
    assert.equal(url, '/api/ai-directory')
    assert.equal(options?.method, 'GET')
    assert.equal(options?.credentials, 'omit')
    assert.equal(options?.cache, 'no-store')
    return Response.json({ success: true, data: { links } })
  }
}

test('reads the stored order and hidden entries; distinguishes unset, empty and malformed', async () => {
  for (const links of [null, [], [link, DEFAULT_AI_DIRECTORY_LINKS[1]]]) {
    read(links)
    assert.deepEqual(await getDirectorySettings(), links)
  }
  const unicode = { ...link, name: '🌱'.repeat(80) }
  read([unicode])
  assert.deepEqual(await getDirectorySettings(), [unicode])
  read([{ ...link, unexpected: true }])
  await assert.rejects(getDirectorySettings())
  read([{}])
  await assert.rejects(getDirectorySettings())
})

test('writes precisely the community contract and reads back the same empty list', async () => {
  const calls: string[] = []
  api.put = (async (url, body) => {
    calls.push(url)
    assert.deepEqual(body, { links: [] })
    return { data: { success: true, data: { links: [] } } }
  }) as typeof api.put
  read([])
  const fetchRead = globalThis.fetch
  globalThis.fetch = async (...args) => {
    calls.push(String(args[0]))
    return fetchRead(...args)
  }
  assert.deepEqual(await saveDirectorySettings([]), [])
  assert.deepEqual(calls, ['/api/ai-directory/links', '/api/ai-directory'])
})

test('rejects forbidden and failed writes without reading back a false success', async () => {
  let reads = 0
  globalThis.fetch = async () => {
    reads++
    throw new Error('unexpected read')
  }
  api.put = (async () => {
    throw Object.assign(new Error('Forbidden'), { response: { status: 403 } })
  }) as typeof api.put
  await assert.rejects(saveDirectorySettings([link]), /Forbidden/)
  api.put = (async () => ({
    data: { success: false, message: 'rejected' },
  })) as typeof api.put
  await assert.rejects(saveDirectorySettings([link]), /rejected/)
  assert.equal(reads, 0)
})

test('retains reported values and rejects a missing, changed or failed readback', async () => {
  api.put = (async () => ({
    data: { success: true, data: { links: [link] } },
  })) as typeof api.put
  read([{ ...link, name: 'Another edit' }])
  await assert.rejects(saveDirectorySettings([link]))
  read(null)
  await assert.rejects(saveDirectorySettings([link]))
  globalThis.fetch = async () => new Response(null, { status: 503 })
  await assert.rejects(saveDirectorySettings([link]))
  read([Object.fromEntries(Object.entries(link).reverse())])
  assert.deepEqual(await saveDirectorySettings([link]), [link])
})
