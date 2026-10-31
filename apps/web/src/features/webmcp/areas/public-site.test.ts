/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

import { publicSiteTools } from './public-site'

test('public developer summary lists distinct public, Session and OAuth catalog contracts', async () => {
  const tools = publicSiteTools({
    router: {
      navigate: async () => undefined,
      subscribe: () => () => undefined,
    },
  })
  const developer = tools.find((tool) => tool.name === 'lmm_developers_read')
  assert.ok(developer)
  const result = await developer.execute(
    {},
    { signal: new AbortController().signal }
  )
  const content = JSON.stringify(result)
  assert.match(content, /\/core\/v1\/pricing\/public/)
  assert.match(content, /\/core\/v1\/pricing\/session/)
  assert.match(content, /\/api\/oauth2\/catalog/)
  assert.doesNotMatch(content, /"path":"\/api\/pricing"|DSH/)
})

const originalAdapter = api.defaults.adapter
const originalAuth = useAuthStore.getState().auth
const scope = { userId: 7, sessionId: 'webmcp-release-session', role: 1 }

function signIn() {
  const now = Math.floor(Date.now() / 1000)
  useAuthStore.getState().auth.setBundle({
    access_token: 'synthetic-release-token',
    token_type: 'Bearer',
    access_expires_at: now + 3600,
    user: {
      id: scope.userId,
      username: 'synthetic-release-reader',
      role: scope.role,
    },
    session: {
      sid: scope.sessionId,
      current: true,
      login_method: 'password',
      ip: '',
      user_agent: '',
      created_at: now - 60,
      last_active_at: now,
      expires_at: now + 7200,
    },
  })
}

function releaseTool() {
  const tool = publicSiteTools({
    router: {
      navigate: async () => undefined,
      subscribe: () => () => undefined,
    },
  }).find((entry) => entry.name === 'lmm_release_notes_read')
  assert.ok(tool)
  return tool
}

afterEach(() => {
  api.defaults.adapter = originalAdapter
  useAuthStore.setState({ auth: originalAuth })
})

test('release note reads bind the signed-in scope and propagate the tool abort signal', async () => {
  signIn()
  const controller = new AbortController()
  const note = {
    id: 17,
    version: 'v1.2.3',
    revision: 1,
    content: 'Synthetic release body',
    published_at: 1_800_000_000,
    published_by: 1,
  }
  let requests = 0
  api.defaults.adapter = async (config) => {
    requests++
    assert.equal(config.method, 'get')
    assert.equal(config.url, '/api/release-notes/latest')
    assert.equal(config.authScope?.userId, scope.userId)
    assert.equal(config.authScope?.sessionId, scope.sessionId)
    assert.equal(config.signal, controller.signal)
    return {
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: { success: true, data: note },
    }
  }
  assert.deepEqual(
    await releaseTool().execute({}, { signal: controller.signal }),
    {
      available: true,
      version: note.version,
      revision: note.revision,
      published_at: new Date(note.published_at * 1000).toISOString(),
      content: note.content,
      content_was_clipped: false,
    }
  )
  assert.equal(requests, 1)
})

test('release note reads reject anonymous or incomplete sessions before dispatch', async () => {
  let requests = 0
  api.defaults.adapter = async (config) => {
    requests++
    return {
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: { success: true, data: null },
    }
  }
  for (const missing of ['user', 'session', 'accessToken'] as const) {
    signIn()
    const auth = useAuthStore.getState().auth
    useAuthStore.setState({ auth: { ...auth, [missing]: null } })
    await assert.rejects(
      releaseTool().execute({}, { signal: new AbortController().signal }),
      /Sign in first/
    )
  }
  assert.equal(requests, 0)
})

test('failed release note reads reject instead of reporting no unread release', async () => {
  signIn()
  api.defaults.adapter = async (config) => ({
    config,
    status: 200,
    statusText: 'OK',
    headers: {},
    data: { success: false, message: 'Synthetic release-note failure' },
  })
  await assert.rejects(
    releaseTool().execute({}, { signal: new AbortController().signal })
  )
})
