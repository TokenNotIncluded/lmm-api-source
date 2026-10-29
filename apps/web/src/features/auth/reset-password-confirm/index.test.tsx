/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
// @ts-expect-error Bun's test module is available only in the test runtime.
import { mock } from 'bun:test'
import assert from 'node:assert/strict'
import { after, afterEach, beforeEach, test } from 'node:test'

import { AxiosError } from 'axios'
import { Window } from 'happy-dom'
import type { ReactNode } from 'react'

const dom = new Window({ url: 'https://console.example.test/user/reset' })
dom.document.write('<!doctype html><html><head></head><body></body></html>')
const originalGlobals = new Map<string, PropertyDescriptor | undefined>()
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLInputElement',
  'HTMLButtonElement',
  'SVGElement',
  'Node',
  'Element',
  'Event',
  'MouseEvent',
  'CustomEvent',
  'MutationObserver',
  'ResizeObserver',
  'requestAnimationFrame',
  'cancelAnimationFrame',
  'getComputedStyle',
  'matchMedia',
  'customElements',
  'localStorage',
  'sessionStorage',
] as const) {
  originalGlobals.set(key, Object.getOwnPropertyDescriptor(globalThis, key))
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: dom[key],
  })
}
originalGlobals.set(
  'IS_REACT_ACT_ENVIRONMENT',
  Object.getOwnPropertyDescriptor(globalThis, 'IS_REACT_ACT_ENVIRONMENT')
)
Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
  configurable: true,
  value: true,
})

mock.module('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}))
mock.module('@/features/auth/auth-layout', () => ({
  AuthLayout: ({ children }: { children: ReactNode }) => (
    <main>{children}</main>
  ),
}))

const { act, StrictMode } = await import('react')
const { createRoot } = await import('react-dom/client')
const { api } = await import('@/lib/api')
const { resetPasswordFormSchema } = await import('@/features/auth/constants')
const { ResetPasswordConfirm } = await import('./index')
const originalAdapter = api.defaults.adapter
let root: ReturnType<typeof createRoot> | undefined
let host: HTMLDivElement
let responseData: unknown
let responseStatus = 200
const requests: unknown[] = []
const email = 'synthetic+reset@example.test'
const token = '0123456789abcdef0123456789abcdef'
const flush = () => new Promise<void>((resolve) => setTimeout(resolve, 0))

beforeEach(() => {
  requests.length = 0
  responseStatus = 200
  responseData = {
    success: true,
    data: { password_changed: true, login_required: true },
  }
  host = document.createElement('div')
  document.body.appendChild(host)
  api.defaults.adapter = async (config) => {
    requests.push({ url: config.url, payload: JSON.parse(config.data) })
    const response = {
      config,
      status: responseStatus,
      statusText: '',
      headers: {},
      data: responseData,
    }
    if (responseStatus >= 400) {
      throw new AxiosError(
        'Synthetic unavailable',
        'ERR_BAD_RESPONSE',
        config,
        undefined,
        response
      )
    }
    return response
  }
})

afterEach(async () => {
  await act(async () => root?.unmount())
  root = undefined
  host.remove()
  api.defaults.adapter = originalAdapter
  delete window.__lmmEmailReset
})

after(async () => {
  await dom.happyDOM.abort()
  for (const [key, descriptor] of originalGlobals) {
    if (descriptor) Object.defineProperty(globalThis, key, descriptor)
    else Reflect.deleteProperty(globalThis, key)
  }
})

async function render(valid = true) {
  if (valid) window.__lmmEmailReset = { email, token }
  root = createRoot(host)
  await act(async () => {
    root?.render(
      <StrictMode>
        <ResetPasswordConfirm />
      </StrictMode>
    )
    await flush()
  })
}

async function fill(name: string, value: string) {
  const input = host.querySelector(`input[name="${name}"]`)
  assert.ok(input instanceof HTMLInputElement)
  const setter = Object.getOwnPropertyDescriptor(
    HTMLInputElement.prototype,
    'value'
  )?.set
  assert.ok(setter)
  await act(async () => {
    setter.call(input, value)
    input.dispatchEvent(new Event('input', { bubbles: true }))
    await flush()
  })
}

async function submit() {
  const form = host.querySelector('form')
  assert.ok(form)
  await act(async () => {
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
    await flush()
  })
}

test('missing bootstrap credentials remain invalid and never request a reset', async () => {
  await render(false)
  assert.match(host.textContent ?? '', /Invalid reset link/)
  assert.equal(host.querySelector('form'), null)
  assert.deepEqual(requests, [])
})

test('StrictMode preserves historical link credentials, validates confirmation, then submits only on explicit action', async () => {
  await render()
  assert.equal(window.__lmmEmailReset, undefined)
  assert.deepEqual(
    requests,
    [],
    'opening a GET link must not consume the token'
  )
  assert.equal(host.querySelectorAll('input[type="password"]').length, 2)
  assert.equal(
    host.querySelectorAll('input[autocomplete="new-password"]').length,
    2
  )
  await fill('password', 'short')
  await fill('confirmPassword', 'short')
  await submit()
  assert.match(host.textContent ?? '', /Password must be between 8 and 20/)
  assert.deepEqual(requests, [])
  await fill('password', ' chosen password ')
  await fill('confirmPassword', 'different password')
  await submit()
  assert.match(host.textContent ?? '', /Passwords don't match/)
  assert.deepEqual(requests, [])
  await fill('confirmPassword', ' chosen password ')
  await submit()
  assert.deepEqual(requests, [
    {
      url: '/api/user/reset',
      payload: { email, token, new_password: ' chosen password ' },
    },
  ])
  assert.match(host.textContent ?? '', /auth.resetPasswordConfirm.success/)
  assert.equal(host.querySelectorAll('input').length, 0)
  assert.doesNotMatch(host.textContent ?? '', /chosen password|clipboard/)
  assert.equal(localStorage.length, 0)
  assert.equal(sessionStorage.length, 0)
})

test('password limits count Unicode characters, preserve spaces and reject mismatched confirmation', () => {
  for (const password of [
    'a'.repeat(8),
    'a'.repeat(20),
    '😀'.repeat(8),
    '😀'.repeat(20),
    ' abcdef ',
  ]) {
    assert.equal(
      resetPasswordFormSchema.safeParse({ password, confirmPassword: password })
        .success,
      true
    )
  }
  for (const password of [
    '',
    'a'.repeat(7),
    'a'.repeat(21),
    '😀'.repeat(7),
    '😀'.repeat(21),
  ]) {
    assert.equal(
      resetPasswordFormSchema.safeParse({ password, confirmPassword: password })
        .success,
      false
    )
  }
  assert.equal(
    resetPasswordFormSchema.safeParse({
      password: 'chosen-password',
      confirmPassword: 'other-password',
    }).success,
    false
  )
})

for (const failure of ['legacy string', 'business failure', 'HTTP 503']) {
  test(`${failure} stays an error without password display or automatic fallback`, async () => {
    responseData =
      failure === 'legacy string'
        ? { success: true, data: 'synthetic-legacy-plaintext' }
        : {
            success: false,
            message: 'Synthetic token detail must not be shown',
          }
    if (failure === 'HTTP 503') responseStatus = 503
    await render()
    await fill('password', 'chosen-password')
    await fill('confirmPassword', 'chosen-password')
    await submit()
    assert.equal(requests.length, 1)
    assert.match(host.textContent ?? '', /Reset failed/)
    assert.doesNotMatch(
      host.textContent ?? '',
      /auth.resetPasswordConfirm.success|synthetic-legacy-plaintext|Synthetic token detail/
    )
  })
}
