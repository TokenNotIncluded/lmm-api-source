/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { after, afterEach, test } from 'node:test'

import { Window } from 'happy-dom'

import type { InvitationAction } from './assistant-invitation-contract'

const dom = new Window({
  url: 'https://console.example.test/',
  settings: {
    fetch: {
      interceptor: {
        beforeAsyncRequest: async () => {
          throw new Error('Uncontrolled network forbidden')
        },
        beforeSyncRequest: () => {
          throw new Error('Uncontrolled network forbidden')
        },
      },
    },
  },
})
const originals = new Map<string, PropertyDescriptor | undefined>()
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLButtonElement',
  'Node',
  'Element',
  'Event',
  'MouseEvent',
  'PointerEvent',
  'FocusEvent',
  'CustomEvent',
  'MutationObserver',
  'ResizeObserver',
  'requestAnimationFrame',
  'cancelAnimationFrame',
  'getComputedStyle',
  'localStorage',
  'sessionStorage',
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
const React = await import('react')
const { act } = React
const { createRoot } = await import('react-dom/client')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { useAuthStore } = await import('@/stores/auth-store')
const { AssistantInvitationCard } = await import('./assistant-invitation-card')
const i18n = createInstance()
await i18n.use(initReactI18next).init({
  lng: 'en',
  fallbackLng: 'en',
  resources: { en: { translation: {} } },
})
const initialFetch = globalThis.fetch
const roots: Array<ReturnType<typeof createRoot>> = []
let serial = 0
function actor(id = 7, token = 'synthetic-original', sid = 'original') {
  useAuthStore.getState().auth.setBundle({
    access_token: token,
    token_type: 'Bearer',
    access_expires_at: 2_000_000_000,
    user: { id, username: 'synthetic-user', role: 1 },
    session: {
      sid,
      current: true,
      login_method: 'password',
      ip: null,
      user_agent: null,
      created_at: 1,
      last_active_at: 1,
      expires_at: 2_000_000_000,
    },
  })
}
function fixture() {
  const action: InvitationAction = {
    type: 'invitation_confirmation',
    tool: 'send_invitation',
    confirmation_id: (++serial).toString(16).padStart(64, '0'),
    operation_id: `original-invitation-operation-${serial}`,
    actor_user_id: 7,
    actor_credential_sha256: createHash('sha256')
      .update('synthetic-original')
      .digest('hex'),
    expires_at: Math.floor(Date.now() / 1000) + 120,
    recipient: 'guest@example.invalid',
  }
  const view = () => ({
    id: action.confirmation_id,
    operation_id: action.operation_id,
    tool: 'send_invitation',
    browser_url: `/api/user/assistant/confirmations/${'c'.repeat(64)}`,
    payload: {
      kind: 'invitation_email_v1',
      email: action.recipient,
      invitation_code: 'Ab12',
    },
    expires_at: action.expires_at,
    consumed_at: null as number | null,
    human_confirmed: false,
    dispatch_started: false,
  })
  return { action, view }
}
async function until(check: () => boolean, label: string) {
  for (let i = 0; i < 60 && !check(); i++) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 5))
    })
  }
  assert.ok(check(), label)
}
async function mount(action: InvitationAction, disabled = false) {
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  roots.push(root)
  await act(async () => {
    root.render(
      <I18nextProvider i18n={i18n}>
        <AssistantInvitationCard action={action} disabled={disabled} />
      </I18nextProvider>
    )
  })
  return host
}
function button(host: HTMLElement, text: string) {
  const b = Array.from(host.querySelectorAll('button')).find(
    (x) => x.textContent === text
  )
  assert.ok(b, 'expected fixed button')
  return b
}
afterEach(async () => {
  for (const root of roots.splice(0)) await act(async () => root.unmount())
  document.body.replaceChildren()
  useAuthStore.getState().auth.reset()
  globalThis.fetch = initialFetch
})
after(async () => {
  await dom.happyDOM.close()
  for (const [name, descriptor] of originals) {
    if (descriptor) Object.defineProperty(globalThis, name, descriptor)
    else Reflect.deleteProperty(globalThis, name)
  }
})

test('browser card uses the actual confirmation URL then one explicit send with unchanged session', async () => {
  actor()
  const f = fixture()
  let approved = false,
    posts = 0,
    reads = 0
  globalThis.fetch = async (input, init) => {
    assert.equal(
      String(input),
      `/api/assistant/services/confirmations/${
        f.action.confirmation_id
      }${init?.method === 'POST' ? '/apply' : ''}`
    )
    assert.equal(init?.credentials, 'include')
    assert.equal(init?.cache, 'no-store')
    assert.ok(
      new Headers(init?.headers).get('Authorization') ===
        'Bearer synthetic-original'
    )
    if (init?.method === 'POST') {
      posts++
      assert.equal(init.body, '{}')
      return Response.json({
        success: true,
        data: {
          ok: true,
          applied: true,
          status: 'accepted_by_smtp',
          smtp_accepted: true,
          smtp_code: 250,
          delivery_confirmed: false,
          retry_safe: false,
          operation_id: f.action.operation_id,
          confirmation_id: f.action.confirmation_id,
          proof_receipt_id: 'synthetic-browser-proof',
        },
      })
    }
    reads++
    return Response.json({
      success: true,
      data: { ...f.view(), human_confirmed: approved },
    })
  }
  const host = await mount(f.action)
  await until(() => !!host.querySelector('a'), 'read-only pending link appears')
  assert.ok(host.textContent?.includes('Ab12'))
  assert.equal(posts, 0)
  assert.equal(
    host.querySelector('a')?.getAttribute('href'),
    f.view().browser_url
  )
  assert.equal(
    host.querySelector('a')?.getAttribute('rel'),
    'noopener noreferrer'
  )
  assert.equal(button(host, 'Send invitation once').disabled, true)
  approved = true
  await act(async () => button(host, 'Check invitation status').click())
  await until(
    () => !button(host, 'Send invitation once').disabled,
    'real approval readback enables button'
  )
  assert.equal(posts, 0)
  await act(async () => button(host, 'Send invitation once').click())
  await until(
    () => !!host.textContent?.includes('Delivery is not confirmed.'),
    'SMTP acceptance remains distinct from delivery'
  )
  await act(async () => button(host, 'Send invitation once').click())
  assert.equal(posts, 1)
  assert.equal(reads, 2)
})
test('session refresh aborts a late read and leaves no recipient or send control enabled', async () => {
  actor()
  const f = fixture()
  let release!: (r: Response) => void
  let started = false,
    posts = 0
  globalThis.fetch = async (_input, init) => {
    if (init?.method === 'POST') {
      posts++
      throw new Error('must not send')
    }
    started = true
    return new Promise<Response>((resolve) => {
      release = resolve
    })
  }
  const host = await mount(f.action)
  await until(() => started, 'read started')
  await act(async () => {
    actor(7, 'synthetic-refreshed', 'original')
    release(
      Response.json({
        success: true,
        data: { ...f.view(), human_confirmed: true },
      })
    )
  })
  await until(
    () => !!host.textContent?.includes('unavailable for the current session'),
    'stale view discarded'
  )
  assert.equal(host.textContent?.includes('guest@example.invalid'), false)
  assert.equal(host.querySelector('a'), null)
  assert.equal(button(host, 'Send invitation once').disabled, true)
  assert.equal(posts, 0)
})

test('disabled cards perform no read and clear receipt details when capability changes', async () => {
  actor()
  const f = fixture()
  let reads = 0
  globalThis.fetch = async (_input, init) => {
    assert.equal(init?.method, 'GET')
    reads++
    return Response.json({ success: true, data: f.view() })
  }
  const host = await mount(f.action, true)
  const root = roots.at(-1)
  assert.ok(root)
  const render = async (disabled: boolean) => {
    await act(async () => {
      root.render(
        <I18nextProvider i18n={i18n}>
          <AssistantInvitationCard action={f.action} disabled={disabled} />
        </I18nextProvider>
      )
    })
  }
  assert.equal(reads, 0)
  assert.equal(button(host, 'Send invitation once').disabled, true)
  assert.equal(button(host, 'Check invitation status').disabled, true)
  await render(false)
  await until(() => !!host.querySelector('a'), 'fresh read enables review')
  assert.equal(reads, 1)
  assert.equal(button(host, 'Send invitation once').disabled, true)
  await render(true)
  assert.equal(host.textContent?.includes(f.action.recipient), false)
  assert.equal(host.querySelector('a'), null)
  assert.equal(button(host, 'Send invitation once').disabled, true)
  assert.equal(button(host, 'Check invitation status').disabled, true)
  assert.equal(reads, 1)
})
