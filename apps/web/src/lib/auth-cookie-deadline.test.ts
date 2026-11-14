/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, test } from 'node:test'

import type { AxiosAdapter, AxiosResponse } from 'axios'

import { executeAssistantUserAction } from '@/features/assistant/api'
import { login, login2fa, logout } from '@/features/auth/api'
import { updateUserProfile } from '@/features/profile/api'
import { useAuthStore, type AuthBundle } from '@/stores/auth-store'

import {
  api,
  setup2FA,
  enable2FA,
  disable2FA,
  regenerate2FABackupCodes,
} from './api'
import {
  applyAuthBundle,
  clearAuthentication,
  refreshAuthentication,
  setDevelopmentAuthRefreshAdapter,
} from './auth-session'

const originalWindow = Object.getOwnPropertyDescriptor(globalThis, 'window')
const originalNavigator = Object.getOwnPropertyDescriptor(
  globalThis,
  'navigator'
)
const originalSecureContext = Object.getOwnPropertyDescriptor(
  globalThis,
  'isSecureContext'
)
const storage = new Map<string, string>()
Object.defineProperty(globalThis, 'isSecureContext', {
  configurable: true,
  value: true,
})
Object.defineProperty(globalThis, 'navigator', {
  configurable: true,
  value: {
    locks: {
      async request<T>(
        _name: string,
        _options: unknown,
        operation: () => Promise<T>
      ) {
        return operation()
      },
    },
  },
})
const originalFetch = globalThis.fetch
Object.defineProperty(globalThis, 'window', {
  configurable: true,
  value: {
    localStorage: {
      getItem: (key: string) => storage.get(key) ?? null,
      setItem: (key: string, value: string) => storage.set(key, value),
      removeItem: (key: string) => storage.delete(key),
    },
    location: {
      href: 'https://deadline.example.test/',
      origin: 'https://deadline.example.test',
    },
  },
})
globalThis.fetch = (async () =>
  Response.json({
    success: true,
    data: { revoked_sid: 'synthetic', current: true },
  })) as typeof fetch
const original = api.defaults.adapter
const bundle: AuthBundle = {
  access_token: 'synthetic',
  token_type: 'Bearer',
  access_expires_at: Math.floor(Date.now() / 1000) + 600,
  user: { id: 42, username: 'synthetic', role: 1 },
  session: {
    sid: 'synthetic',
    current: true,
    login_method: 'password',
    ip: '',
    user_agent: '',
    created_at: 1,
    last_active_at: 1,
    expires_at: 2_000_000_000,
  },
}
after(() => {
  globalThis.fetch = originalFetch
  if (originalWindow) {
    Object.defineProperty(globalThis, 'window', originalWindow)
  } else Reflect.deleteProperty(globalThis, 'window')
  for (const [key, descriptor] of [
    ['navigator', originalNavigator],
    ['isSecureContext', originalSecureContext],
  ] as const) {
    if (descriptor) Object.defineProperty(globalThis, key, descriptor)
    else Reflect.deleteProperty(globalThis, key)
  }
  api.defaults.adapter = original
  setDevelopmentAuthRefreshAdapter(original as AxiosAdapter)
  clearAuthentication(false, 'idle')
})
test('every participating Cookie writer configures the same finite Axios deadline', async () => {
  const observed: Array<{ url: string; timeout: number | undefined }> = []
  const adapter: AxiosAdapter = async (config): Promise<AxiosResponse> => {
    observed.push({ url: String(config.url), timeout: config.timeout })
    return {
      config,
      status: 200,
      statusText: 'synthetic',
      headers: {},
      data: { success: true, data: bundle },
    }
  }
  api.defaults.adapter = adapter
  setDevelopmentAuthRefreshAdapter(adapter)
  clearAuthentication(false, 'idle')
  await login({ username: 'synthetic', password: 'synthetic-password' })
  useAuthStore.getState().auth.setPending2FAFlowToken('synthetic-flow')
  await login2fa({ code: '123456', flow_token: 'synthetic-flow' })
  await setup2FA()
  await enable2FA('123456')
  await disable2FA('123456')
  await regenerate2FABackupCodes('123456')
  await updateUserProfile({
    original_password: 'synthetic',
    password: 'synthetic-new',
  })
  await executeAssistantUserAction(
    {
      type: 'user_password_change',
      requires_confirmation: true,
      target_user_id: 42,
      target_username: 'synthetic',
      target_display_name: 'Synthetic',
      target_role: 1,
      target_group: 'default',
      target_is_self: true,
    },
    { currentPassword: 'synthetic', newPassword: 'synthetic-new' }
  )
  await logout()
  applyAuthBundle(bundle, false)
  await refreshAuthentication()
  assert.equal(observed.length, 10)
  assert.deepEqual(
    observed.map((entry) => entry.timeout),
    Array(10).fill(10_000)
  )
})
