/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
// @ts-expect-error Bun test runtime only.
import { mock } from 'bun:test'
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import axios, { AxiosError, type InternalAxiosRequestConfig } from 'axios'
import { Window } from 'happy-dom'
import { toast } from 'sonner'

const dom = new Window({ url: 'https://console.example.test/sign-in' })
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLInputElement',
  'HTMLTextAreaElement',
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
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: dom[key],
  })
}
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
mock.module('@/hooks/use-status', () => ({
  useStatus: () => ({ status: null }),
}))
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { AccountAppealForm } = await import('./account-appeal-form')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { setDevelopmentAuthRefreshAdapter } = await import('@/lib/auth-session')
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
const originalAdapter = api.defaults.adapter
const originalToast = toast.error
const notices: unknown[] = []
let refreshes = 0
let root: ReturnType<typeof createRoot> | undefined
const receipt = {
  id: 71,
  status: 'pending',
  reason: 'Original server reason',
  created_at: 1,
  reviewed_at: null,
}
const context = {
  username: 'github-only',
  public_reason: 'Published account restriction',
  public_reason_available: true,
  expires_at: Math.floor(Date.now() / 1000) + 600,
  request: null,
}
function respond(
  config: InternalAxiosRequestConfig,
  data: unknown,
  status = 200
) {
  assert.equal(config.withCredentials, true)
  assert.equal(config.authMode, undefined)
  assert.equal(config.skipAuthRefresh, true)
  assert.equal(config.skipErrorHandler, true)
  assert.equal(config.skipBusinessError, true)
  const result = { config, data, status, statusText: 'synthetic', headers: {} }
  if (status >= 400) {
    throw new AxiosError(
      'private transport diagnostic',
      'ERR_BAD_RESPONSE',
      config,
      undefined,
      result
    )
  }
  return result
}
async function mount(refreshKey = 0) {
  if (!root) {
    const host = document.createElement('div')
    document.body.append(host)
    root = createRoot(host)
  }
  toast.error = (message) => {
    notices.push(message)
    return 'appeal-error'
  }
  setDevelopmentAuthRefreshAdapter(async () => {
    refreshes++
    throw new Error('must not refresh')
  })
  await act(async () => {
    root?.render(
      <I18nextProvider i18n={i18n}>
        <AccountAppealForm refreshKey={refreshKey} />
      </I18nextProvider>
    )
    await new Promise((resolve) => setTimeout(resolve, 20))
  })
}
async function fill(id: string, value: string) {
  const input = document.getElementById(id)
  assert.ok(
    input instanceof HTMLInputElement || input instanceof HTMLTextAreaElement
  )
  const prototype =
    input instanceof HTMLInputElement
      ? dom.HTMLInputElement.prototype
      : dom.HTMLTextAreaElement.prototype
  await act(async () => {
    Object.getOwnPropertyDescriptor(prototype, 'value')?.set?.call(input, value)
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
}
function button(text: string) {
  const item = Array.from(document.querySelectorAll('button')).find(
    (item) => item.textContent === text
  )
  assert.ok(item, text)
  return item
}
async function click(text: string) {
  await act(async () => {
    button(text).click()
    await new Promise((resolve) => setTimeout(resolve, 10))
  })
}
afterEach(async () => {
  if (root) await act(async () => root?.unmount())
  root = undefined
  document.body.replaceChildren()
  api.defaults.adapter = originalAdapter
  toast.error = originalToast
  setDevelopmentAuthRefreshAdapter(axios.getAdapter(originalAdapter))
  useAuthStore.getState().auth.reset('complete')
  assert.equal(refreshes, 0)
  refreshes = 0
  notices.length = 0
})
after(() => {
  mock.restore()
  dom.close()
})

test('OAuth-only identity submits only reason and shows the existing server receipt', async () => {
  const writes: unknown[] = []
  api.defaults.adapter = async (config) => {
    if (config.method === 'get') {
      return respond(config, {
        success: true,
        data: { ...context, reason: 'INTERNAL', admin_note: 'SECRET' },
      })
    }
    writes.push(JSON.parse(config.data))
    return respond(config, {
      success: true,
      data: { ...receipt, already_pending: true, admin_note: 'SECRET' },
    })
  }
  await mount()
  assert.match(document.body.textContent ?? '', /github-only/)
  assert.match(document.body.textContent ?? '', /Published account restriction/)
  assert.equal(document.querySelector('input[type=password]'), null)
  await fill('appeal-reason', 'new explanation')
  await click('Send appeal')
  assert.deepEqual(writes, [{ reason: 'new explanation' }])
  assert.match(document.body.textContent ?? '', /Original server reason/)
  assert.match(document.body.textContent ?? '', /Pending review/)
  assert.doesNotMatch(
    document.body.textContent ?? '',
    /INTERNAL|SECRET|new explanation/
  )
  assert.equal(
    Array.from(document.querySelectorAll('button')).some(
      (item) => item.textContent === 'Send appeal'
    ),
    false
  )
  assert.equal(useAuthStore.getState().auth.session, null)
  assert.deepEqual(notices, [])
})

for (const status of [
  'pending',
  'approved',
  'rejected',
  'superseded',
] as const) {
  test(`context displays actual ${status} status and unpublished reason safely`, async () => {
    api.defaults.adapter = async (config) =>
      respond(config, {
        success: true,
        data: {
          ...context,
          public_reason: 'PRIVATE DO NOT DISPLAY',
          public_reason_available: false,
          request: { ...receipt, status },
        },
      })
    await mount()
    assert.match(
      document.body.textContent ?? '',
      /No public reason has been provided/
    )
    assert.doesNotMatch(document.body.textContent ?? '', /PRIVATE/)
    assert.match(
      document.body.textContent ?? '',
      new RegExp(
        status === 'pending'
          ? 'Pending review'
          : status[0].toUpperCase() + status.slice(1)
      )
    )
    if (status === 'approved') {
      assert.match(document.body.textContent ?? '', /Sign in again to continue/)
    }
    assert.equal(
      document.getElementById('appeal-reason') !== null,
      status === 'rejected' || status === 'superseded'
    )
    assert.equal(useAuthStore.getState().auth.accessToken, null)
  })
}

test('receipt status and original reason override a submitted draft, including approval', async () => {
  api.defaults.adapter = async (config) =>
    respond(config, {
      success: true,
      data:
        config.method === 'get'
          ? context
          : { ...receipt, status: 'approved', already_pending: false },
    })
  await mount()
  await fill('appeal-reason', 'draft explanation')
  await click('Send appeal')
  assert.match(document.body.textContent ?? '', /Approved/)
  assert.match(document.body.textContent ?? '', /Original server reason/)
  assert.doesNotMatch(document.body.textContent ?? '', /Pending review/)
})

test('anonymous password appeal preserves the legacy body and has one safe error notification', async () => {
  const writes: unknown[] = []
  api.defaults.adapter = async (config) => {
    if (config.method === 'get') return respond(config, {}, 401)
    writes.push(JSON.parse(config.data))
    return respond(config, {
      success: false,
      message: 'PRIVATE backend detail',
    })
  }
  await mount()
  await click('Account disabled? Submit an appeal')
  await fill('appeal-username', 'legacy-user')
  await fill('appeal-password', 'local-password')
  await fill('appeal-reason', 'legacy explanation')
  await click('Send appeal')
  assert.deepEqual(writes, [
    {
      username: 'legacy-user',
      password: 'local-password',
      reason: 'legacy explanation',
    },
  ])
  assert.deepEqual(notices, ['Unable to submit appeal'])
})

test('password failure refresh signal reloads Cookie context and removes password inputs', async () => {
  let verified = false
  let reads = 0
  api.defaults.adapter = async (config) => {
    reads++
    return respond(
      config,
      { success: true, data: context },
      verified ? 200 : 401
    )
  }
  await mount()
  await click('Account disabled? Submit an appeal')
  assert.ok(document.getElementById('appeal-password'))
  verified = true
  await mount(1)
  assert.equal(reads, 2)
  assert.equal(document.getElementById('appeal-password'), null)
  assert.match(document.body.textContent ?? '', /github-only/)
})

test('expired Cookie submission is not replayed and asks for new identity verification', async () => {
  let posts = 0
  api.defaults.adapter = async (config) => {
    if (config.method === 'get') {
      return respond(config, { success: true, data: context })
    }
    posts++
    return respond(config, { message: 'PRIVATE' }, 401)
  }
  await mount()
  await fill('appeal-reason', 'explanation')
  await click('Send appeal')
  assert.equal(posts, 1)
  assert.match(document.body.textContent ?? '', /Appeal verification expired/)
  assert.equal(button('Send appeal').disabled, true)
  assert.deepEqual(notices, [])
})

test('a late receipt cannot appear under a newly verified identity', async () => {
  let release!: (value: unknown) => void
  const pendingReceipt = new Promise((resolve) => {
    release = resolve
  })
  let username = 'first-identity'
  api.defaults.adapter = async (config) => {
    if (config.method === 'get') {
      return respond(config, { success: true, data: { ...context, username } })
    }
    const data = await pendingReceipt
    return respond(config, data)
  }
  await mount()
  await fill('appeal-reason', 'first identity explanation')
  await click('Send appeal')
  username = 'second-identity'
  await mount(1)
  await act(async () => {
    release({ success: true, data: { ...receipt, already_pending: false } })
    await new Promise((resolve) => setTimeout(resolve, 10))
  })
  assert.match(document.body.textContent ?? '', /second-identity/)
  assert.doesNotMatch(
    document.body.textContent ?? '',
    /Original server reason|Pending review|first identity explanation/
  )
})
