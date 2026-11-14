/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, beforeEach, test } from 'node:test'

import { Window } from 'happy-dom'

const dom = new Window({ url: 'https://console.example.test/settings' })
const originals = new Map<string, PropertyDescriptor | undefined>()
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLInputElement',
  'HTMLButtonElement',
  'Node',
  'Element',
  'Event',
  'MouseEvent',
  'PointerEvent',
  'MutationObserver',
  'ResizeObserver',
  'getComputedStyle',
  'matchMedia',
  'customElements',
] as const) {
  originals.set(key, Object.getOwnPropertyDescriptor(globalThis, key))
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: dom[key],
  })
}
originals.set(
  'IS_REACT_ACT_ENVIRONMENT',
  Object.getOwnPropertyDescriptor(globalThis, 'IS_REACT_ACT_ENVIRONMENT')
)
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { HumanGrantSection } = await import('./humangrant-section')
const { useAuthStore } = await import('@/stores/auth-store')
const { HUMAN_GRANT_SETTINGS } = await import('@/features/auth/lib/humangrant')
const { toast } = await import('sonner')
const notifications: unknown[] = []
const toastMethods = ['error', 'success', 'info', 'warning'] as const
const originalToasts = new Map(toastMethods.map((key) => [key, toast[key]]))
for (const key of toastMethods) {
  Reflect.set(toast, key, (...args: unknown[]) => {
    notifications.push(args)
    return 0
  })
}
const i18n = createInstance()
await i18n.use(initReactI18next).init({ lng: 'en', resources: {} })

function actor(id = 41, sid = 'session-A') {
  // Complete synthetic local fixture, never a server-issued proof or a latch override.
  useAuthStore.getState().auth.setBundle({
    access_token: `synthetic-${id}-${sid}`,
    token_type: 'Bearer',
    access_expires_at: 2_000_000_000,
    user: { id, username: `synthetic-${id}`, role: 100 },
    session: {
      sid,
      current: true,
      login_method: 'password',
      ip: '',
      user_agent: '',
      created_at: 1,
      last_active_at: 1,
      expires_at: 2_000_000_000,
    },
  })
}
type Pending = {
  init: RequestInit
  signal: AbortSignal
  resolve: (response: Response) => void
  reject: (error: Error) => void
}
const pending: Pending[] = []
const originalFetch = globalThis.fetch
let root: ReturnType<typeof createRoot> | undefined
let host: HTMLDivElement

beforeEach(() => {
  pending.length = 0
  notifications.length = 0
  actor()
  globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
    assert.equal(
      input,
      HUMAN_GRANT_SETTINGS,
      'no uncontrolled endpoint or real network'
    )
    assert.ok(init?.signal)
    assert.equal(init.credentials, 'same-origin')
    assert.equal(init.redirect, 'error')
    assert.equal(init.cache, 'no-store')
    assert.equal(new Headers(init.headers).has('Authorization'), false)
    // Deliberately ignore abort: late transport/body completion must still be safe.
    return new Promise<Response>((resolve, reject) =>
      pending.push({
        init,
        signal: init.signal as AbortSignal,
        resolve,
        reject,
      })
    )
  }) as typeof fetch
  host = document.createElement('div')
  document.body.append(host)
})
afterEach(async () => {
  await unmount()
  assert.deepEqual(notifications, [], 'no stale global notification')
  globalThis.fetch = originalFetch
  host.remove()
})
after(() => {
  for (const [key, value] of originalToasts) Reflect.set(toast, key, value)
  dom.close()
  for (const [key, descriptor] of originals) {
    if (descriptor) Object.defineProperty(globalThis, key, descriptor)
    else Reflect.deleteProperty(globalThis, key)
  }
})
async function mount() {
  const mounted = createRoot(host)
  root = mounted
  await act(async () =>
    mounted.render(
      <I18nextProvider i18n={i18n}>
        <HumanGrantSection />
      </I18nextProvider>
    )
  )
}
async function unmount() {
  const mounted = root
  if (!mounted) return
  await act(async () => mounted.unmount())
  root = undefined
}
function checkbox() {
  const node = host.querySelector<HTMLInputElement>('input[type=checkbox]')
  assert.ok(node)
  return node
}
function button(text: string) {
  const node = Array.from(host.querySelectorAll('button')).find(
    (element) => element.textContent === text
  )
  assert.ok(node, `missing button ${text}`)
  return node
}
function status() {
  return host.querySelector('[role=status]')?.textContent ?? ''
}
async function respond(
  index: number,
  enabled: boolean,
  csrf = `proof-${index}`
) {
  assert.ok(pending[index], `missing request ${index}`)
  await act(async () =>
    pending[index].resolve(Response.json({ enabled, csrf }))
  )
}
async function aba() {
  const obsolete = pending.at(-1)
  assert.ok(obsolete)
  await act(async () => {
    actor(99, 'session-B')
    assert.equal(
      obsolete.signal.aborted,
      true,
      'external store event aborts before React renders'
    )
    actor()
  })
}

test('batched A→B→A retires the old read and saves only with the freshly read CSRF proof', async () => {
  await mount()
  assert.equal(pending.length, 1)
  assert.equal(host.querySelector('section')?.getAttribute('aria-busy'), 'true')
  assert.equal(button('Save').disabled, true)
  await aba()
  assert.equal(pending.length, 2, 'returning A must acquire another proof')
  await respond(0, true, 'obsolete-proof')
  assert.equal(checkbox().checked, false)
  assert.equal(button('Save').disabled, true)
  assert.equal(status(), '')
  await respond(1, false, 'fresh-A-proof')
  assert.equal(button('Save').disabled, false)
  await act(async () => checkbox().click())
  assert.equal(checkbox().checked, true)
  await act(async () => button('Save').click())
  assert.equal(pending.length, 3)
  assert.equal(pending[2].init.method, 'PUT')
  assert.equal(
    new Headers(pending[2].init.headers).get('X-HG-CSRF'),
    'fresh-A-proof'
  )
  assert.equal(pending[2].init.body, JSON.stringify({ enabled: true }))
  await respond(2, true)
  assert.equal(pending.length, 4, 'real write helper reconciles with a GET')
  assert.equal(pending[3].init.method, undefined)
  await respond(3, true, 'rotated-A-proof')
  assert.equal(status(), 'HumanGrant settings saved.')
  assert.equal(pending.filter(({ init }) => init.method === 'PUT').length, 1)
})

test('a late save receipt after A→B→A cannot restore settings, announce success or reconcile the old scope', async () => {
  await mount()
  await respond(0, true, 'old-write-proof')
  await act(async () => button('Save').click())
  assert.equal(pending[1].init.method, 'PUT')
  await aba()
  assert.equal(pending.length, 3)
  await respond(1, true)
  assert.equal(
    pending.length,
    3,
    'aborted old PUT must not issue a reconciliation GET'
  )
  assert.equal(button('Save').disabled, true)
  assert.equal(checkbox().checked, false)
  assert.equal(status(), '')
  await respond(2, false, 'new-scope-proof')
  assert.equal(checkbox().checked, false)
  assert.equal(button('Save').disabled, false)
  assert.equal(status(), '')
})

test('manual reload removes the draft and keeps saving disabled until a new valid read', async () => {
  await mount()
  await respond(0, false)
  await act(async () => checkbox().click())
  assert.equal(checkbox().checked, true)
  await act(async () => button('Reload HumanGrant settings').click())
  assert.equal(pending.length, 2)
  assert.equal(
    checkbox().checked,
    false,
    'old draft is cleared while read is pending'
  )
  assert.equal(button('Save').disabled, true)
  await act(async () => pending[1].resolve(Response.json({}, { status: 503 })))
  assert.match(status(), /unavailable/)
  assert.equal(button('Save').disabled, true)
  await act(async () => button('Reload HumanGrant settings').click())
  assert.equal(status(), '')
  await respond(2, false, 'reload-proof')
  assert.equal(button('Save').disabled, false)
  assert.equal(
    pending.some(({ init }) => init.method === 'PUT'),
    false
  )
})

test('unmounted read rejection and save completion never notify or affect a remounted form', async () => {
  await mount()
  await unmount()
  assert.equal(pending[0].signal.aborted, true)
  await mount()
  await act(async () => pending[0].reject(new Error('obsolete read failure')))
  assert.equal(status(), '')
  assert.equal(button('Save').disabled, true)
  await respond(1, true, 'mounted-proof')
  await act(async () => button('Save').click())
  await unmount()
  assert.equal(pending[2].signal.aborted, true)
  await mount()
  await respond(2, true)
  assert.equal(pending.length, 4)
  assert.equal(status(), '')
  assert.equal(button('Save').disabled, true)
  await respond(3, false, 'remounted-proof')
  assert.equal(checkbox().checked, false)
  assert.equal(status(), '')
  assert.deepEqual(notifications, [])
})
