/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { Window } from 'happy-dom'

import type { UserProfile } from '../types'

const dom = new Window({
  url: 'https://console.example.test/profile?tab=account',
})
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLInputElement',
  'SVGElement',
  'Node',
  'Element',
  'Event',
  'CustomEvent',
  'MutationObserver',
  'getComputedStyle',
  'requestAnimationFrame',
  'cancelAnimationFrame',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: dom[key],
  })
}
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { applyAuthBundle } = await import('@/lib/auth-session')
const { useAuthStore } = await import('@/stores/auth-store')
const { installSupportedAuthBrowser } =
  await import('@/test-support/supported-auth-browser')
const { ProfileIdentityCard } = await import('./profile-identity-card')
installSupportedAuthBrowser({ window: dom })
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
const adapter = api.defaults.adapter
const previousAuth = useAuthStore.getState().auth
afterEach(() => {
  api.defaults.adapter = adapter
  useAuthStore.setState({ auth: previousAuth })
})
after(() => dom.close())

const profile: UserProfile = {
  id: 501,
  username: 'synthetic-profile',
  display_name: 'Original name',
  role: 1,
  group: '',
  quota: 0,
  used_quota: 0,
  request_count: 0,
  status: 1,
  aff_count: 0,
  aff_quota: 0,
  aff_history_quota: 0,
  created_time: 1,
}
function login() {
  applyAuthBundle(
    {
      access_token: 'synthetic-profile-token',
      token_type: 'Bearer',
      access_expires_at: 2000000000,
      user: profile,
      session: {
        sid: 'synthetic-profile-session',
        current: true,
        login_method: 'password',
        ip: '127.0.0.1',
        user_agent: 'synthetic',
        created_at: 1,
        last_active_at: 1,
        expires_at: 2000000000,
      },
    },
    false
  )
}
function change(input: HTMLInputElement, value: string) {
  Object.getOwnPropertyDescriptor(
    HTMLInputElement.prototype,
    'value'
  )?.set?.call(input, value)
  input.dispatchEvent(new Event('input', { bubbles: true }))
}

test('identity form saves a changed name and confirms it through the self endpoint', async () => {
  login()
  const calls: string[] = []
  api.defaults.adapter = async (config) => {
    calls.push(config.method ?? '')
    return {
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data:
        config.method === 'put'
          ? { success: true }
          : { success: true, data: { ...profile, display_name: 'New name' } },
    }
  }
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  let refreshed = 0
  try {
    await act(async () =>
      root.render(
        <I18nextProvider i18n={i18n}>
          <ProfileIdentityCard
            profile={profile}
            loading={false}
            onProfileUpdate={() => {
              refreshed++
            }}
          />
        </I18nextProvider>
      )
    )
    const input = host.querySelector<HTMLInputElement>(
      '#profile-display-name-input'
    )
    const submit = host.querySelector<HTMLButtonElement>('button[type=submit]')
    assert.ok(input && submit)
    assert.equal(submit.disabled, true)
    await act(async () => change(input, 'New name'))
    assert.equal(submit.disabled, false)
    await act(async () => {
      host
        .querySelector('form')
        ?.dispatchEvent(
          new Event('submit', { bubbles: true, cancelable: true })
        )
      await new Promise((resolve) => setTimeout(resolve, 15))
    })
    assert.deepEqual(calls, ['put', 'get'])
    assert.equal(refreshed, 1)
    assert.equal(useAuthStore.getState().auth.user?.display_name, 'New name')
    assert.match(host.textContent ?? '', /Profile updated successfully/)
  } finally {
    await act(async () => root.unmount())
    host.remove()
  }
})

test('identity form rejects names above the byte limit and keeps the draft on server failure', async () => {
  login()
  let calls = 0
  api.defaults.adapter = async (config) => {
    calls++
    return {
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: { success: false },
    }
  }
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  try {
    await act(async () =>
      root.render(
        <I18nextProvider i18n={i18n}>
          <ProfileIdentityCard
            profile={profile}
            loading={false}
            onProfileUpdate={() => {
              throw new Error('failed save cannot refresh')
            }}
          />
        </I18nextProvider>
      )
    )
    const input = host.querySelector<HTMLInputElement>(
      '#profile-display-name-input'
    )
    assert.ok(input)
    await act(async () => change(input, '界'.repeat(86)))
    await act(async () => {
      host
        .querySelector('form')
        ?.dispatchEvent(
          new Event('submit', { bubbles: true, cancelable: true })
        )
    })
    assert.equal(calls, 0)
    assert.match(
      host.querySelector('[role=alert]')?.textContent ?? '',
      /256 UTF-8 bytes/
    )
    await act(async () => change(input, 'Retry name'))
    await act(async () => {
      host
        .querySelector('form')
        ?.dispatchEvent(
          new Event('submit', { bubbles: true, cancelable: true })
        )
      await new Promise((resolve) => setTimeout(resolve, 15))
    })
    assert.equal(calls, 1)
    assert.equal(input.value, 'Retry name')
    assert.match(
      host.querySelector('[role=alert]')?.textContent ?? '',
      /Could not confirm/
    )
    assert.equal(
      useAuthStore.getState().auth.user?.display_name,
      'Original name'
    )
  } finally {
    await act(async () => root.unmount())
    host.remove()
  }
})
