/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import { useAuthStore, useCredentialStore } from '@/stores/auth-store'

import { getCommonHeaders } from './auth-session'
import { api } from './http-client'

const adapter = api.defaults.adapter
afterEach(() => {
  api.defaults.adapter = adapter
  useCredentialStore.getState().auth.reset()
})
test('preview rejects business writes, private reads and streaming credentials before transport', async () => {
  useCredentialStore
    .getState()
    .auth.setUser({ id: 42, role: 100, username: 'owner' })
  useAuthStore.getState().startPreview('/users')
  let sent = 0
  api.defaults.adapter = async (config) => {
    sent++
    return { config, headers: {}, status: 200, statusText: 'OK', data: {} }
  }
  for (const [method, url] of [
    ['post', '/api/assistant/chat'],
    ['post', '/api/remote-control/v1/pi/sessions/1/messages'],
    ['post', '/api/user/pay'],
    ['get', '/api/users'],
    ['delete', '/api/user/self'],
  ] as const) {
    await assert.rejects(api.request({ method, url }))
  }
  assert.throws(() => getCommonHeaders())
  assert.equal(sent, 0)
  const self = await api.get('/api/user/self')
  assert.equal(self.data.data.id, -1)
  assert.equal(sent, 0)
})
