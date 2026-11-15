/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import { useAuthStore, useCredentialStore } from './auth-store'

afterEach(() => useCredentialStore.getState().auth.reset())
test('preview projects a clean L0 while preserving real credentials', () => {
  useCredentialStore.getState().auth.setUser({
    id: 42,
    username: 'owner-secret',
    role: 100,
    email: 'private@example.com',
    quota: 123,
  })
  useCredentialStore.setState((state) => ({
    auth: { ...state.auth, accessToken: 'real-token' },
  }))
  useAuthStore.getState().startPreview('/users?search=x#row')
  assert.equal(useAuthStore.getState().auth.user?.role, 1)
  assert.equal(useAuthStore.getState().auth.user?.email, undefined)
  assert.equal(useAuthStore.getState().auth.user?.quota, undefined)
  assert.equal(useCredentialStore.getState().auth.user?.id, 42)
  assert.equal(useCredentialStore.getState().auth.accessToken, 'real-token')
  assert.equal(
    useAuthStore.getState().preview?.returnHref,
    '/users?search=x#row'
  )
  useAuthStore.getState().stopPreview()
  assert.equal(useAuthStore.getState().auth.user?.id, 42)
})
test('only super administrators enter; account changes and logout clear preview', () => {
  const auth = useCredentialStore.getState().auth
  auth.setUser({ id: 1, username: 'user', role: 1 })
  useAuthStore.getState().startPreview('/')
  assert.equal(useAuthStore.getState().preview, null)
  auth.setUser({ id: 1, username: 'owner', role: 100 })
  useAuthStore.getState().startPreview('/')
  auth.setUser({ id: 2, username: 'other', role: 100 })
  assert.equal(useAuthStore.getState().preview, null)
  useAuthStore.getState().startPreview('/')
  auth.reset()
  assert.equal(useAuthStore.getState().preview, null)
})
