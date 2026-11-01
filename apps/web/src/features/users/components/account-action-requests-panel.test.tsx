/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
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

const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { AccountActionRequestsPanel } =
  await import('./account-action-requests-panel')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { setDevelopmentAuthRefreshAdapter } = await import('@/lib/auth-session')
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
const originalAdapter = api.defaults.adapter
const originalError = toast.error
const originalSuccess = toast.success
const notices: unknown[] = []
const writes: unknown[] = []
const reads: Array<{ status: string; page: number; page_size: number }> = []
let refreshes = 0
let root: ReturnType<typeof createRoot> | undefined
const request = {
  id: 71,
  target_user_id: 7,
  requested_by_user_id: 7,
  kind: 'appeal',
  status: 'pending',
  reason: 'Please restore my account',
  admin_user_id: 0,
  admin_note: '',
  created_at: 1,
  reviewed_at: 0,
  target_username: 'github-user',
  target_email: '',
  requested_by_username: 'github-user',
  requested_by_email: '',
  revision: '9007199254740993',
  auth_version: '9007199254740995',
}
function respond(
  config: InternalAxiosRequestConfig,
  data: unknown,
  status = 200
) {
  assert.equal(config.withCredentials, true)
  assert.equal(config.skipAuthRefresh, true)
  assert.equal(config.skipErrorHandler, true)
  assert.equal(config.skipBusinessError, true)
  const result = { config, data, status, statusText: 'synthetic', headers: {} }
  if (status >= 400) {
    throw new AxiosError(
      'PRIVATE diagnostic',
      'ERR_BAD_RESPONSE',
      config,
      undefined,
      result
    )
  }
  return result
}
async function mount() {
  const host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
  toast.error = (message) => {
    notices.push(message)
    return 'error'
  }
  toast.success = () => 'success'
  setDevelopmentAuthRefreshAdapter(async () => {
    refreshes++
    throw new Error('must not refresh')
  })
  await act(async () => {
    root?.render(
      <I18nextProvider i18n={i18n}>
        <AccountActionRequestsPanel />
      </I18nextProvider>
    )
    await new Promise((resolve) => setTimeout(resolve, 20))
  })
}
async function click(text: string) {
  const item = Array.from(document.querySelectorAll('button')).find(
    (item) => item.textContent?.trim() === text
  )
  assert.ok(item, text)
  await act(async () => {
    item.click()
    await new Promise((resolve) => setTimeout(resolve, 20))
  })
}
async function filter(status: string) {
  const select = document.querySelector('select')
  assert.ok(select)
  await act(async () => {
    select.value = status
    select.dispatchEvent(new Event('change', { bubbles: true }))
    await new Promise((resolve) => setTimeout(resolve, 20))
  })
}
async function fillNote(value: string) {
  const input = document.querySelector('textarea')
  assert.ok(input)
  await act(async () => {
    Object.getOwnPropertyDescriptor(
      dom.HTMLTextAreaElement.prototype,
      'value'
    )?.set?.call(input, value)
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
}
afterEach(async () => {
  if (root) await act(async () => root?.unmount())
  root = undefined
  document.body.replaceChildren()
  api.defaults.adapter = originalAdapter
  toast.error = originalError
  toast.success = originalSuccess
  setDevelopmentAuthRefreshAdapter(axios.getAdapter(originalAdapter))
  useAuthStore.getState().auth.reset('complete')
  assert.equal(refreshes, 0)
  refreshes = 0
  notices.length = 0
  writes.length = 0
  reads.length = 0
})
after(() => dom.close())

test('CAS conflict preserves the note, reloads facts, and only an explicit second action reviews the new revision', async () => {
  let conflict = false
  let approved = false
  api.defaults.adapter = async (config) => {
    if (config.method === 'get') {
      reads.push(config.params)
      const current = {
        ...request,
        revision: conflict ? '9007199254740994' : request.revision,
        status: approved ? 'approved' : 'pending',
      }
      return respond(config, {
        success: true,
        data: [current],
        pagination: { page: config.params.page, page_size: 20, total: 1 },
      })
    }
    writes.push(JSON.parse(config.data))
    if (!conflict) {
      conflict = true
      return respond(config, {}, 409)
    }
    approved = true
    return respond(config, {
      success: true,
      data: { ...request, status: 'approved' },
    })
  }
  await mount()
  await fillNote('keep my review note')
  await click('Approve and unblock')
  assert.deepEqual(writes, [
    {
      note: 'keep my review note',
      expected_revision: '9007199254740993',
      expected_auth_version: '9007199254740995',
    },
  ])
  assert.equal(document.querySelector('textarea')?.value, 'keep my review note')
  assert.equal(document.querySelector('select')?.value, 'all')
  assert.equal(reads.length, 2)
  assert.deepEqual(notices, [
    'This request changed. Review the refreshed details before trying again.',
  ])
  await click('Approve and unblock')
  assert.deepEqual(writes[1], {
    note: 'keep my review note',
    expected_revision: '9007199254740994',
    expected_auth_version: '9007199254740995',
  })
  assert.equal(reads.length, 3, 'successful review reloads the list')
  assert.match(document.body.textContent ?? '', /Approved/)
  assert.equal(document.querySelector('textarea'), null)
})

test('pagination visits the server pages and changing status resets to page one', async () => {
  api.defaults.adapter = async (config) => {
    reads.push(config.params)
    return respond(config, {
      success: true,
      data: [
        {
          ...request,
          id: config.params.page,
          target_username: `page-${config.params.page}`,
        },
      ],
      pagination: { page: config.params.page, page_size: 20, total: 121 },
    })
  }
  await mount()
  assert.deepEqual(reads[0], { status: 'pending', page: 1, page_size: 20 })
  await click('Next')
  assert.match(document.body.textContent ?? '', /page-2/)
  await filter('rejected')
  assert.deepEqual(reads.at(-1), { status: 'rejected', page: 1, page_size: 20 })
  await click('Next')
  await click('Previous')
  assert.deepEqual(reads.at(-1), { status: 'rejected', page: 1, page_size: 20 })
})

test('superseded requests cannot be reviewed, including after a CAS conflict', async () => {
  let changed = false
  api.defaults.adapter = async (config) => {
    if (config.method === 'get') {
      return respond(config, {
        success: true,
        data: [{ ...request, status: changed ? 'superseded' : 'pending' }],
        pagination: { page: 1, page_size: 20, total: 1 },
      })
    }
    changed = true
    writes.push(JSON.parse(config.data))
    return respond(config, {}, 409)
  }
  await mount()
  await fillNote('keep note')
  await click('Reject')
  assert.equal(writes.length, 1)
  assert.match(document.body.textContent ?? '', /Superseded/)
  assert.equal(document.querySelector('textarea')?.value, 'keep note')
  assert.equal(document.querySelector('textarea')?.readOnly, true)
  assert.equal(
    Array.from(document.querySelectorAll('button')).some(
      (item) => item.textContent?.trim() === 'Reject'
    ),
    false
  )
})
