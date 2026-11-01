/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { Window } from 'happy-dom'

const dom = new Window({ url: 'https://console.example.test/users' })
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLButtonElement',
  'HTMLInputElement',
  'HTMLTextAreaElement',
  'HTMLLabelElement',
  'SVGElement',
  'Node',
  'Element',
  'Event',
  'MouseEvent',
  'PointerEvent',
  'FocusEvent',
  'CustomEvent',
  'MutationObserver',
  'ResizeObserver',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: dom[key],
  })
}
Object.defineProperties(globalThis, {
  requestAnimationFrame: {
    configurable: true,
    value: (callback: FrameRequestCallback) => setTimeout(() => callback(0), 0),
  },
  cancelAnimationFrame: {
    configurable: true,
    value: (handle: number) => clearTimeout(handle),
  },
  getComputedStyle: {
    configurable: true,
    value: dom.getComputedStyle.bind(dom),
  },
  IS_REACT_ACT_ENVIRONMENT: { configurable: true, value: true },
})
after(() => dom.close())

const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { readNativeUsersScope } = await import('./access')
const { NativeUserSecurity } = await import('./security-editor')
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
const originalAdapter = api.defaults.adapter
const previousAuth = useAuthStore.getState().auth
const userId = '9007199254740993'
const snapshot = {
  contract: 'core.admin-user-security.v1',
  user_id: userId,
  auth_version: '9007199254740997',
  active: true,
  account_active: true,
  mfa_enabled: false,
}
let root: ReturnType<typeof createRoot> | undefined
const writes: unknown[] = []
let updated = 0
const onUpdated = () => {
  updated++
}
const onAccessDenied = () => {
  throw new Error('unexpected denied access')
}
async function pause() {
  await new Promise((resolve) => setTimeout(resolve, 20))
}
async function click(text: string) {
  const button = Array.from(document.querySelectorAll('button')).find(
    (value) => value.textContent?.trim() === text
  )
  assert.ok(button, text)
  await act(async () => {
    button.click()
    await pause()
  })
}
async function fill(id: string, value: string) {
  const input = document.getElementById(id)
  assert.ok(input instanceof HTMLInputElement)
  await act(async () => {
    Object.getOwnPropertyDescriptor(
      dom.HTMLInputElement.prototype,
      'value'
    )?.set?.call(input, value)
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
}
async function choose(action: string) {
  const select = document.querySelector('select')
  assert.ok(select)
  await act(async () => {
    select.value = action
    select.dispatchEvent(new Event('change', { bubbles: true }))
    await pause()
  })
}
async function mount() {
  useAuthStore.getState().auth.setBundle({
    access_token: 'synthetic-native-security',
    token_type: 'Bearer',
    access_expires_at: 2000000000,
    user: { id: 501, role: 10, username: 'synthetic-admin' },
    session: {
      sid: 'synthetic-native-security-session',
      current: true,
      login_method: 'password',
      ip: '',
      user_agent: '',
      created_at: 1,
      last_active_at: 1,
      expires_at: 2000000000,
    },
  })
  const scope = readNativeUsersScope()
  assert.ok(scope)
  api.defaults.adapter = async (config) => {
    assert.equal(config.skipAuthRefresh, true)
    assert.equal(
      config.url,
      `/core/v1/admin/user-security/${userId}${config.method === 'post' ? '/actions' : ''}`
    )
    if (config.method === 'post') writes.push(JSON.parse(String(config.data)))
    return {
      config,
      data:
        config.method === 'post'
          ? { changed: true, user: { ...snapshot, active: false } }
          : snapshot,
      status: 200,
      statusText: 'OK',
      headers: {},
    }
  }
  const host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
  await act(async () => {
    root?.render(
      <I18nextProvider i18n={i18n}>
        <NativeUserSecurity
          scope={scope}
          userId={userId}
          onUpdated={onUpdated}
          onAccessDenied={onAccessDenied}
        />
      </I18nextProvider>
    )
    await pause()
  })
  await click('Security')
  await act(pause)
}
afterEach(async () => {
  if (root) {
    await act(async () => {
      root?.unmount()
      await pause()
    })
  }
  root = undefined
  document.body.replaceChildren()
  api.defaults.adapter = originalAdapter
  useAuthStore.setState({ auth: previousAuth })
  writes.length = 0
  updated = 0
})

test('native Security disable keeps mandatory internal evidence separate from the optional public explanation', async () => {
  await mount()
  assert.match(
    document.body.textContent ?? '',
    /Internal reason \(administrators only\)/
  )
  assert.match(document.body.textContent ?? '', /Public reason \(optional\)/)
  await fill(
    `security-public-reason-${userId}`,
    '  Published restriction explanation  '
  )
  const confirm = document.querySelector<HTMLButtonElement>(
    'button[type=submit]'
  )
  assert.ok(confirm)
  assert.equal(
    confirm.disabled,
    true,
    'public explanation cannot replace the mandatory internal reason'
  )
  await fill(`security-reason-${userId}`, 'Private moderation evidence')
  await click('Confirm')
  assert.deepEqual(writes, [
    {
      expected_auth_version: snapshot.auth_version,
      action: 'disable',
      reason: 'Private moderation evidence',
      public_reason: 'Published restriction explanation',
    },
  ])
  assert.equal(updated, 1)
})

test('leaving public explanation empty omits public_reason and never copies the private reason', async () => {
  await mount()
  await fill(`security-reason-${userId}`, 'Private moderation evidence')
  await fill(`security-public-reason-${userId}`, '   ')
  await click('Confirm')
  assert.deepEqual(writes, [
    {
      expected_auth_version: snapshot.auth_version,
      action: 'disable',
      reason: 'Private moderation evidence',
    },
  ])
})

test('changing security action clears and hides the public explanation and does not send it for MFA reset', async () => {
  await mount()
  await fill(`security-reason-${userId}`, 'Private recovery evidence')
  await fill(
    `security-public-reason-${userId}`,
    'Published restriction explanation'
  )
  await choose('reset_mfa')
  assert.equal(
    document.getElementById(`security-public-reason-${userId}`),
    null
  )
  await choose('disable')
  const input = document.getElementById(`security-public-reason-${userId}`)
  assert.ok(input instanceof HTMLInputElement)
  assert.equal(input.value, '')
  await choose('reset_mfa')
  await click('Confirm')
  assert.deepEqual(writes, [
    {
      expected_auth_version: snapshot.auth_version,
      action: 'reset_mfa',
      reason: 'Private recovery evidence',
    },
  ])
})
