/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { Window } from 'happy-dom'

test('a reset request can be resent with the same address after the Core issue window', async () => {
  const dom = new Window({
    url: 'https://console.example.test/forgot-password',
  })
  const originalGlobals = new Map<string, PropertyDescriptor | undefined>()
  for (const key of [
    'window',
    'document',
    'navigator',
    'HTMLElement',
    'HTMLInputElement',
    'Event',
    'Node',
    'Element',
    'MutationObserver',
    'IS_REACT_ACT_ENVIRONMENT',
  ] as const) {
    originalGlobals.set(key, Object.getOwnPropertyDescriptor(globalThis, key))
    Object.defineProperty(globalThis, key, {
      configurable: true,
      value:
        key === 'IS_REACT_ACT_ENVIRONMENT'
          ? true
          : key === 'window'
            ? dom
            : dom[key],
    })
  }
  const { act } = await import('react')
  const { createRoot } = await import('react-dom/client')
  const { QueryClient, QueryClientProvider } =
    await import('@tanstack/react-query')
  const { createInstance } = await import('i18next')
  const { I18nextProvider } = await import('react-i18next')
  const { api } = await import('@/lib/api')
  const { ForgotPasswordForm } = await import('./forgot-password-form')
  const { PASSWORD_RESET_COUNTDOWN } = await import('@/features/auth/constants')
  const i18n = createInstance()
  await i18n.init({ lng: 'en', resources: { en: { translation: {} } } })
  const client = new QueryClient()
  client.setQueryData(['status'], { turnstile_check: false })
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  const adapter = api.defaults.adapter
  const address = 'recovery-fixture@example.test'
  const attempts: string[] = []
  let releaseFirstResponse!: () => void
  const firstResponse = new Promise<void>((resolve) => {
    releaseFirstResponse = resolve
  })
  api.defaults.adapter = async (config) => {
    assert.equal(config.url, '/api/identity/email/password-reset/request')
    attempts.push(JSON.parse(config.data).email)
    if (attempts.length === 1) await firstResponse
    return {
      config,
      headers: {},
      status: 202,
      statusText: 'Accepted',
      data: { success: true, data: { status: 'accepted_if_eligible' } },
    }
  }
  let tick: (() => void) | undefined
  Object.defineProperty(dom, 'setInterval', {
    configurable: true,
    value: (callback: () => void, milliseconds: number) => {
      assert.equal(milliseconds, 1000)
      tick = callback
      return 1
    },
  })
  const advance = async (seconds: number) => {
    const callback = tick
    assert.ok(callback)
    await act(async () => {
      for (let index = 0; index < seconds; index++) callback()
    })
  }
  try {
    await act(async () =>
      root.render(
        <QueryClientProvider client={client}>
          <I18nextProvider i18n={i18n}>
            <ForgotPasswordForm />
          </I18nextProvider>
        </QueryClientProvider>
      )
    )
    const email = host.querySelector<HTMLInputElement>('input[type="email"]')
    const button = host.querySelector<HTMLButtonElement>(
      'button[type="submit"]'
    )
    const form = host.querySelector('form')
    assert.ok(email && button && form)
    const setValue = Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      'value'
    )?.set
    assert.ok(setValue)
    await act(async () => {
      setValue.call(email, address)
      email.dispatchEvent(new Event('input', { bubbles: true }))
    })
    const dispatchSubmit = () =>
      form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
    const submit = async () => {
      await act(async () => {
        dispatchSubmit()
        await new Promise((resolve) => setTimeout(resolve, 0))
      })
    }
    await act(async () => {
      dispatchSubmit()
      dispatchSubmit()
      await new Promise((resolve) => setTimeout(resolve, 0))
    })
    assert.deepEqual(attempts, [address])
    assert.equal(button.disabled, true)
    await act(async () => {
      releaseFirstResponse()
      await new Promise((resolve) => setTimeout(resolve, 0))
    })
    assert.equal(email.value, address)
    assert.equal(button.disabled, true)
    assert.match(button.textContent ?? '', /60s/)
    await advance(30)
    await submit()
    assert.equal(button.disabled, true)
    assert.deepEqual(attempts, [address])
    await advance(29)
    assert.equal(button.disabled, true)
    await advance(1)
    assert.equal(button.disabled, false)
    await submit()
    assert.deepEqual(attempts, [address, address])
    assert.equal(PASSWORD_RESET_COUNTDOWN, 30)
  } finally {
    releaseFirstResponse()
    await act(async () => root.unmount())
    client.clear()
    api.defaults.adapter = adapter
    await dom.close()
    for (const [key, descriptor] of originalGlobals) {
      if (descriptor) Object.defineProperty(globalThis, key, descriptor)
      else Reflect.deleteProperty(globalThis, key)
    }
  }
})
