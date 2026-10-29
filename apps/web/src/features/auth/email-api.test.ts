/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import axios, { AxiosError } from 'axios'

import {
  bindEmail,
  sendEmailVerification as requestBindCode,
} from '@/features/profile/api'
import { api } from '@/lib/api'
import {
  clearAuthentication,
  setDevelopmentAuthRefreshAdapter,
} from '@/lib/auth-session'
import { useAuthStore, type AuthBundle } from '@/stores/auth-store'

import {
  register,
  resetPassword,
  sendEmailVerification,
  sendPasswordResetEmail,
} from './api'

const account: AuthBundle = {
  access_token: 'synthetic-existing-token',
  token_type: 'Bearer',
  access_expires_at: 1,
  user: { id: 42, username: 'synthetic', role: 1 },
  session: {
    sid: 'a'.repeat(64),
    current: true,
    login_method: 'password',
    ip: '',
    user_agent: '',
    created_at: 1,
    last_active_at: 1,
    expires_at: 2_000_000_000,
  },
}
const email = 'synthetic@example.test'
const token = '0123456789abcdef0123456789abcdef'

test('new email requests use explicit POST purposes and preserve authentication boundaries', async () => {
  const originalAdapter = api.defaults.adapter
  const requests: Array<{ url: string | undefined; data: unknown }> = []
  let refreshes = 0
  clearAuthentication(false, 'idle')
  useAuthStore.getState().auth.setBundle(account)
  setDevelopmentAuthRefreshAdapter(async () => {
    refreshes++
    throw new Error('Email writes must not refresh or replay')
  })
  api.defaults.adapter = async (config) => {
    requests.push({ url: config.url, data: JSON.parse(config.data) })
    assert.equal(config.method, 'post')
    assert.equal(config.params, undefined)
    assert.equal(config.skipAuthRefresh, true)
    if (
      JSON.parse(config.data).purpose === 'bind' ||
      config.url === '/api/oauth/email/bind'
    ) {
      assert.equal(
        config.headers.get('Authorization'),
        'Bearer synthetic-existing-token'
      )
      assert.equal(config.headers.get('X-Auth-Session'), account.session.sid)
      assert.deepEqual(config.authScope, {
        userId: account.user.id,
        sessionId: account.session.sid,
      })
    } else {
      assert.equal(config.authMode, 'anonymous')
      assert.equal(config.withCredentials, false)
      assert.equal(config.headers.has('Authorization'), false)
    }
    return {
      config,
      status: 202,
      statusText: 'Accepted',
      headers: {},
      data: {
        success: true,
        message: '',
        data: { status: 'accepted_if_eligible' },
      },
    }
  }
  try {
    await sendEmailVerification(email, 'synthetic-turnstile')
    await sendPasswordResetEmail(email)
    await requestBindCode(email)
    await bindEmail(email, '09ab3f')
    const registration = {
      username: 'synthetic',
      password: 'chosen-password',
      email,
      verification_code: '09ab3f',
      accepted_legal: true,
    }
    await register(registration)
    assert.deepEqual(requests, [
      {
        url: '/api/identity/email/verification/request',
        data: { email, purpose: 'register', turnstile: 'synthetic-turnstile' },
      },
      { url: '/api/identity/email/password-reset/request', data: { email } },
      {
        url: '/api/identity/email/verification/request',
        data: { email, purpose: 'bind' },
      },
      { url: '/api/oauth/email/bind', data: { email, code: '09ab3f' } },
      { url: '/api/user/register', data: registration },
    ])
    assert.equal(refreshes, 0)
    assert.equal(useAuthStore.getState().auth.session?.sid, account.session.sid)
  } finally {
    api.defaults.adapter = originalAdapter
    setDevelopmentAuthRefreshAdapter(axios.getAdapter(originalAdapter))
    clearAuthentication(false, 'idle')
  }
})

test('binding rejects account changes and same-SID bearer rotation before dispatch', async () => {
  const originalAdapter = api.defaults.adapter
  let requests = 0
  api.defaults.adapter = async () => {
    requests++
    throw new Error('Superseded email writes must not be dispatched')
  }
  try {
    for (const operation of [
      () => requestBindCode(email),
      () => bindEmail(email, '09ab3f'),
    ]) {
      for (const replacement of [
        { ...account, access_token: 'synthetic-rotated-token' },
        { ...account, session: { ...account.session, sid: 'b'.repeat(64) } },
        { ...account, user: { ...account.user, id: 43 } },
      ]) {
        useAuthStore.getState().auth.setBundle(account)
        const pending = operation()
        useAuthStore.getState().auth.setBundle(replacement)
        await assert.rejects(pending, axios.CanceledError)
      }
      clearAuthentication(false, 'idle')
      await assert.rejects(operation(), axios.CanceledError)
    }
    assert.equal(requests, 0)
  } finally {
    api.defaults.adapter = originalAdapter
    clearAuthentication(false, 'idle')
  }
})

test('email writes never refresh or replay a 401 response', async () => {
  const originalAdapter = api.defaults.adapter
  let requests = 0
  let refreshes = 0
  useAuthStore.getState().auth.setBundle(account)
  setDevelopmentAuthRefreshAdapter(async () => {
    refreshes++
    throw new Error('Email writes must not refresh')
  })
  api.defaults.adapter = async (config) => {
    requests++
    throw new AxiosError(
      'Synthetic unauthorized',
      'ERR_BAD_RESPONSE',
      config,
      undefined,
      {
        config,
        status: 401,
        statusText: '',
        headers: {},
        data: { success: false },
      }
    )
  }
  try {
    for (const operation of [
      () => sendEmailVerification(email),
      () => sendPasswordResetEmail(email),
      () => requestBindCode(email),
      () => bindEmail(email, '09ab3f'),
      () =>
        register({ username: 'synthetic', password: 'chosen-password', email }),
    ]) {
      await assert.rejects(operation(), AxiosError)
    }
    assert.equal(requests, 5)
    assert.equal(refreshes, 0)
    assert.equal(useAuthStore.getState().auth.session?.sid, account.session.sid)
  } finally {
    api.defaults.adapter = originalAdapter
    setDevelopmentAuthRefreshAdapter(axios.getAdapter(originalAdapter))
    clearAuthentication(false, 'idle')
  }
})

test('reset only accepts the new object response and never falls back to legacy requests', async () => {
  const originalAdapter = api.defaults.adapter
  const payload = { email, token, new_password: ' chosen password ' }
  let requests = 0
  let status = 200
  let data: unknown = {
    success: true,
    data: { password_changed: true, login_required: true },
  }
  api.defaults.adapter = async (config) => {
    requests++
    assert.equal(config.method, 'post')
    assert.equal(config.url, '/api/user/reset')
    assert.deepEqual(JSON.parse(config.data), payload)
    assert.equal(config.params, undefined)
    assert.equal(config.authMode, 'anonymous')
    assert.equal(config.skipAuthRefresh, true)
    assert.equal(config.skipBusinessError, true)
    assert.equal(config.skipErrorHandler, true)
    const response = { config, status, statusText: '', headers: {}, data }
    if (status >= 400) {
      throw new AxiosError(
        'Synthetic failure',
        'ERR_BAD_RESPONSE',
        config,
        undefined,
        response
      )
    }
    return response
  }
  try {
    assert.equal((await resetPassword(payload)).success, true)
    for (const incompatible of [
      { success: true, data: 'synthetic-legacy-plaintext' },
      { success: true },
      {
        success: true,
        data: { password_changed: true, login_required: false },
      },
    ]) {
      data = incompatible
      await assert.rejects(resetPassword(payload))
    }
    data = { success: false, message: 'Synthetic unavailable' }
    for (status of [401, 404, 503]) {
      await assert.rejects(resetPassword(payload), AxiosError)
    }
    assert.equal(requests, 7)
  } finally {
    api.defaults.adapter = originalAdapter
  }
})
