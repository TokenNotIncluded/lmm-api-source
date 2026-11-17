/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import type { AxiosRequestConfig, AxiosResponse } from 'axios'

import { api } from '@/lib/api'
import { useAuthStore, type AuthUser } from '@/stores/auth-store'

import { refreshCurrentAccount } from './use-auth-user-refresh'

const originalGet = api.get
const originalAdapter = api.defaults.adapter
const previousAuth = useAuthStore.getState().auth
const user = { id: 41, username: 'test', quota: 10 } as AuthUser

const bundle = (sid: string, available = false) => ({
  access_token: 'local-test-token',
  token_type: 'Bearer',
  access_expires_at: 1900000000,
  user: { ...user, financial_projection_available: available },
  session: {
    sid,
    current: true,
    login_method: 'password',
    ip: '127.0.0.1',
    user_agent: 'local-test',
    created_at: 1,
    last_active_at: 1,
    expires_at: 1900000000,
  },
})

function deferredAccountRead() {
  let resolve: (value: unknown) => void = () => undefined
  let reject: (error: Error) => void = () => undefined
  const response = new Promise<unknown>((resolveResponse, rejectResponse) => {
    resolve = resolveResponse
    reject = rejectResponse
  })
  return {
    response,
    resolve: (available: boolean) =>
      resolve({
        data: {
          success: true,
          data: { ...user, financial_projection_available: available },
        },
      }),
    reject,
  }
}

afterEach(() => {
  api.get = originalGet
  api.defaults.adapter = originalAdapter
  useAuthStore.setState({ auth: previousAuth })
})

test('refreshes the current account after a key mutation', async () => {
  useAuthStore.getState().auth.setUser(user)
  api.get = (async () => ({
    data: { success: true, data: { ...user, quota: 20 } },
  })) as typeof api.get
  await refreshCurrentAccount()
  assert.equal(useAuthStore.getState().auth.user?.quota, 20)
})

test('does not restore the old account if it changes while refreshing', async () => {
  useAuthStore.getState().auth.setUser(user)
  let finish: (value: unknown) => void = () => undefined
  api.get = (() =>
    new Promise<unknown>((resolve) => {
      finish = resolve
    })) as typeof api.get
  const request = refreshCurrentAccount()
  useAuthStore.getState().auth.setUser({ ...user, id: 42 })
  finish({ data: { success: true, data: { ...user, quota: 20 } } })
  assert.equal(await request, null)
  assert.equal(useAuthStore.getState().auth.user?.id, 42)
})

test('a profile refresh cannot overwrite a newer session of the same account', async () => {
  useAuthStore.getState().auth.setBundle(bundle('old-session'))
  let finish: (value: unknown) => void = () => undefined
  api.get = (() =>
    new Promise<unknown>((resolve) => {
      finish = resolve
    })) as typeof api.get
  const request = refreshCurrentAccount()
  useAuthStore.getState().auth.setBundle(bundle('new-session'))
  finish({
    data: { success: true, data: { ...user, financial_projection_available: true } },
  })
  assert.equal(await request, null)
  assert.equal(useAuthStore.getState().auth.session?.sid, 'new-session')
  assert.equal(
    useAuthStore.getState().auth.user?.financial_projection_available,
    false
  )
})

for (const order of ['new-first', 'old-first']) {
  test(`same-account reads retain the new profile result when responses finish ${order}`, async () => {
    useAuthStore.getState().auth.setBundle(bundle('same-session', false))
    const oldRead = deferredAccountRead()
    const newRead = deferredAccountRead()
    let calls = 0
    api.get = (() =>
      ++calls === 1 ? oldRead.response : newRead.response) as typeof api.get
    const oldRequest = refreshCurrentAccount()
    const newRequest = refreshCurrentAccount()
    if (order === 'new-first') {
      newRead.resolve(true)
      assert.equal((await newRequest)?.financial_projection_available, true)
      oldRead.resolve(false)
      assert.equal(await oldRequest, null)
    } else {
      oldRead.resolve(false)
      assert.equal(await oldRequest, null)
      newRead.resolve(true)
      assert.equal((await newRequest)?.financial_projection_available, true)
    }
    assert.equal(
      useAuthStore.getState().auth.user?.financial_projection_available,
      true
    )
  })
}

for (const order of ['failure-first', 'stale-first']) {
  test(`a failed newer refresh preserves confirmed profile data when responses finish ${order}`, async () => {
    useAuthStore.getState().auth.setBundle(bundle('same-session', true))
    const oldRead = deferredAccountRead()
    const newRead = deferredAccountRead()
    let calls = 0
    api.get = (() =>
      ++calls === 1 ? oldRead.response : newRead.response) as typeof api.get
    const oldRequest = refreshCurrentAccount()
    const newRequest = refreshCurrentAccount()
    if (order === 'failure-first') {
      newRead.reject(new Error('HTTP 503'))
      assert.equal(await newRequest, null)
      oldRead.resolve(false)
      assert.equal(await oldRequest, null)
    } else {
      oldRead.resolve(false)
      assert.equal(await oldRequest, null)
      newRead.reject(new Error('HTTP 503'))
      assert.equal(await newRequest, null)
    }
    assert.equal(
      useAuthStore.getState().auth.user?.financial_projection_available,
      true
    )
  })
}

test('a profile refresh sends a fresh identity-scoped GET instead of joining the old HTTP request', async () => {
  useAuthStore.getState().auth.setBundle(bundle('same-http-session', false))
  const reads: Array<{
    config: AxiosRequestConfig
    finish: (available: boolean) => void
  }> = []
  // Preserve the actual HTTP client's duplicate-request and auth interceptors.
  // Only its transport adapter is mocked; no network request is sent.
  api.defaults.adapter = (config) =>
    new Promise<AxiosResponse>((resolve) => {
      reads.push({
        config,
        finish: (available) =>
          resolve({
            status: 200,
            statusText: 'OK',
            headers: {},
            config,
            data: {
              success: true,
              data: { ...user, financial_projection_available: available },
            },
          }),
      })
    })
  const oldRequest = refreshCurrentAccount()
  const newRequest = refreshCurrentAccount()
  for (let tick = 0; tick < 20 && reads.length < 2; tick++) {
    await Promise.resolve()
  }
  assert.equal(
    reads.length,
    2,
    'a completed profile update requires a new physical account read'
  )
  for (const read of reads) {
    assert.equal(read.config.url, '/api/user/self')
    assert.equal(read.config.disableDuplicate, true)
    assert.deepEqual(read.config.authScope, {
      userId: user.id,
      sessionId: 'same-http-session',
    })
  }
  reads[1].finish(true)
  assert.equal((await newRequest)?.financial_projection_available, true)
  reads[0].finish(false)
  assert.equal(await oldRequest, null)
  assert.equal(
    useAuthStore.getState().auth.user?.financial_projection_available,
    true
  )
})

for (const scenario of [
  {
    name: 'a same-session bundle confirms the new projection',
    write: () =>
      useAuthStore
        .getState()
        .auth.setBundle(bundle('same-write-session', true)),
    staleAvailable: false,
    expectedAvailable: true,
  },
  {
    name: 'a same-account user write confirms the new projection',
    write: () =>
      useAuthStore.getState().auth.setUser({
        ...user,
        financial_projection_available: true,
      }),
    staleAvailable: false,
    expectedAvailable: true,
  },
  {
    name: 'reset restores the same account and session without the projection',
    write: () => {
      useAuthStore.getState().auth.reset()
      useAuthStore
        .getState()
        .auth.setBundle(bundle('same-write-session', false))
    },
    staleAvailable: true,
    expectedAvailable: false,
  },
]) {
  test(`a late physical self response is ignored after ${scenario.name}`, async () => {
    useAuthStore.getState().auth.setBundle(bundle('same-write-session', false))
    let dispatched = false
    let finish: (available: boolean) => void = () => undefined
    // Keep the real HTTP client so a same-ID/SID response passes its scope check.
    api.defaults.adapter = (config) =>
      new Promise<AxiosResponse>((resolve) => {
        dispatched = true
        finish = (available) =>
          resolve({
            status: 200,
            statusText: 'OK',
            headers: {},
            config,
            data: {
              success: true,
              data: { ...user, financial_projection_available: available },
            },
          })
      })
    const oldRequest = refreshCurrentAccount()
    for (let tick = 0; tick < 20 && !dispatched; tick++) {
      await Promise.resolve()
    }
    assert.equal(dispatched, true)
    scenario.write()
    finish(scenario.staleAvailable)
    assert.equal(await oldRequest, null)
    assert.equal(useAuthStore.getState().auth.user?.id, user.id)
    assert.equal(
      useAuthStore.getState().auth.session?.sid,
      'same-write-session'
    )
    assert.equal(
      useAuthStore.getState().auth.user?.financial_projection_available,
      scenario.expectedAvailable
    )
  })
}
