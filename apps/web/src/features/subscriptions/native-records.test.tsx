/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, beforeEach, test } from 'node:test'

import { Window } from 'happy-dom'
import type { ReactNode } from 'react'

import type { NativeGrantRecord } from './native-admin-api'

const dom = new Window({
  url: 'https://synthetic.example/subscriptions',
  // This suite exercises forms and recovery, not native animation timing.
  settings: { device: { prefersReducedMotion: 'reduce' } },
})
dom.document.write('<!doctype html><html><head></head><body></body></html>')
for (const name of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLInputElement',
  'HTMLButtonElement',
  'SVGElement',
  'Element',
  'Node',
  'Event',
  'MouseEvent',
  'CustomEvent',
  'MutationObserver',
  'ResizeObserver',
  'DocumentFragment',
  'ShadowRoot',
  'getComputedStyle',
  'requestAnimationFrame',
  'cancelAnimationFrame',
  'sessionStorage',
] as const) {
  Object.defineProperty(globalThis, name, {
    configurable: true,
    value: dom[name],
  })
}
Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
  configurable: true,
  value: true,
})
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { NativeSubscriptionRecords } = await import('./native-records')
const { NativeSubscriptionAdmin } = await import('./native-admin')
const { UserSubscriptionsDialog } =
  await import('./components/dialogs/user-subscriptions-dialog')
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
const adapter = api.defaults.adapter
const auth = useAuthStore.getState().auth
const scope = { userId: 1, sessionId: 'synthetic-subscription-admin' }
const recoveryKey = `lmm-commerce:${scope.userId}:${scope.sessionId}:subscription-invalidation`
const flush = () => new Promise((resolve) => setTimeout(resolve, 20))

function login(role = 100, userId = scope.userId, sid = scope.sessionId) {
  useAuthStore.getState().auth.setBundle({
    user: {
      id: userId,
      role,
      username: 'synthetic-admin',
      wallet_unit: 'core_credit',
    },
    access_token: 'synthetic-access-token',
    token_type: 'Bearer',
    access_expires_at: 9999999999,
    session: {
      sid,
      current: true,
      login_method: 'password',
      ip: null,
      user_agent: null,
      created_at: 1,
      last_active_at: 1,
      expires_at: 9999999999,
    },
  })
}
function record(userId = 52, id = 501): NativeGrantRecord {
  return {
    id,
    user_id: userId,
    username: `synthetic-buyer-${userId}`,
    email: 'synthetic@example.test',
    grant_reference: `legacy-sub:synthetic:${id}`,
    source: 'imported',
    plan: null,
    starts_at: 1700000000,
    ends_at: 4102444800,
    window_start: 1700000000,
    window_end: 4102444800,
    limit_units: '9007199254740993',
    unlimited: false,
    used_units: '65',
    reserved_units: '40',
    active: true,
    status: 'active',
    purchase_id: null,
    refund_journal_id: null,
    renewal_canceled: null,
  }
}
function page(items: NativeGrantRecord[], current = 1, total = items.length) {
  return {
    contract: 'core.subscriptions.v1',
    items,
    total,
    page: current,
    page_size: 20,
  }
}
async function mount(element: ReactNode) {
  const container = document.body.appendChild(document.createElement('div'))
  const root = createRoot(container)
  const render = async (next: ReactNode) => {
    await act(async () => {
      root.render(<I18nextProvider i18n={i18n}>{next}</I18nextProvider>)
      await flush()
    })
    await act(async () => {
      await flush()
    })
  }
  await render(element)
  return {
    render,
    close: async () => {
      await act(async () => root.unmount())
      container.remove()
    },
  }
}
function button(label: string) {
  const found = [...document.querySelectorAll('button')].find(
    (el) => el.textContent?.trim() === label
  )
  assert.ok(found, `Missing button: ${label}`)
  return found
}
async function click(label: string) {
  await act(async () => {
    button(label).click()
    await flush()
  })
}
async function input(selector: string, value: string) {
  const field = document.querySelector(selector)
  assert.ok(field instanceof HTMLInputElement)
  const setValue = Object.getOwnPropertyDescriptor(
    HTMLInputElement.prototype,
    'value'
  )?.set
  assert.ok(setValue)
  await act(async () => {
    setValue.call(field, value)
    field.dispatchEvent(new Event('input', { bubbles: true }))
    await flush()
  })
}
beforeEach(() => {
  dom.sessionStorage.clear()
  login()
})
afterEach(() => {
  api.defaults.adapter = adapter
  useAuthStore.setState({ auth })
  document.body.replaceChildren()
})
after(() => dom.close())

test('invalidation requires a reason, recovers the same command after lost acknowledgement, and rejects an unrelated receipt', async () => {
  const writes: { path: string; body: unknown }[] = []
  let changed = 0
  api.defaults.adapter = async (config) => {
    let data: unknown
    if (config.method === 'get') {
      assert.ok(config.url?.startsWith('/core/v1/subscriptions/admin/records?'))
      data = page([record()])
    } else {
      assert.equal(config.method, 'post')
      assert.equal(
        config.url,
        '/core/v1/subscriptions/admin/grants/501/invalidate'
      )
      const body = JSON.parse(String(config.data))
      const raw = dom.sessionStorage.getItem(recoveryKey)
      assert.ok(raw)
      const saved = JSON.parse(raw)
      assert.deepEqual(
        saved,
        { grant_id: 501, request: body },
        'persist the original command before dispatch'
      )
      assert.ok(config.url)
      writes.push({ path: config.url, body })
      if (writes.length === 1) throw new Error('synthetic acknowledgement lost')
      data = {
        contract: 'core.subscriptions.v1',
        operation_key: body.operation_key,
        subscription_id: 501,
        user_id: writes.length === 2 ? 53 : 52,
        invalidated_at: 1700000001,
        renewal_stopped: false,
      }
    }
    return { config, data, headers: {}, status: 200, statusText: 'OK' }
  }
  const element = (
    <NativeSubscriptionRecords
      scope={scope}
      onChanged={() => {
        changed++
      }}
    />
  )
  let view = await mount(element)
  try {
    assert.match(document.body.textContent ?? '', /Imported subscription/)
    assert.match(document.body.textContent ?? '', /9,007,199,254,740,993/)
    await click('Invalidate')
    assert.equal(button('Confirm').disabled, true)
    assert.equal(writes.length, 0)
    await input(
      '#subscription-invalidation-reason',
      'Synthetic entitlement review'
    )
    await click('Confirm')
    assert.equal(writes.length, 1)
    assert.match(document.body.textContent ?? '', /Result unknown/)
    assert.equal(button('Cancel').disabled, true)
    await view.close()
    view = await mount(element)
    assert.equal(writes.length, 1, 'mount must not dispatch a mutation')
    assert.ok(document.querySelector('[role="dialog"]'))
    await click('Retry saved request')
    assert.equal(writes.length, 2)
    assert.equal(changed, 0, 'a receipt for a different user is not success')
    assert.ok(dom.sessionStorage.getItem(recoveryKey))
    await click('Retry saved request')
    assert.equal(writes.length, 3)
    assert.deepEqual(writes[1], writes[0])
    assert.deepEqual(writes[2], writes[0])
    assert.equal(changed, 1)
    assert.equal(dom.sessionStorage.getItem(recoveryKey), null)
    assert.equal(document.querySelector('[role="dialog"]'), null)
  } finally {
    await view.close()
  }
})

test('ordinary administrators can inspect records without exposing financial plan or invalidation controls', async () => {
  login(10)
  const reads: string[] = []
  api.defaults.adapter = async (config) => {
    assert.equal(config.method, 'get')
    assert.ok(config.url)
    reads.push(config.url)
    assert.ok(config.url?.startsWith('/core/v1/subscriptions/admin/records?'))
    return {
      config,
      data: page([record()]),
      headers: {},
      status: 200,
      statusText: 'OK',
    }
  }
  const view = await mount(<NativeSubscriptionAdmin />)
  try {
    assert.match(document.body.textContent ?? '', /synthetic-buyer-52/)
    assert.match(document.body.textContent ?? '', /Reserved quota/)
    assert.equal(document.querySelector('button[data-value="plans"]'), null)
    assert.ok(
      ![...document.querySelectorAll('button')].some((el) =>
        ['Invalidate', 'Create Plan'].includes(el.textContent?.trim() ?? '')
      )
    )
    assert.equal(reads.length, 1)
  } finally {
    await view.close()
  }
})

test('native user drawer uses the user-filtered Core records and ignores the previous target response', async () => {
  login(10)
  const reads: string[] = []
  let finishOld!: (value: ReturnType<typeof page>) => void
  const old = new Promise<ReturnType<typeof page>>((resolve) => {
    finishOld = resolve
  })
  api.defaults.adapter = async (config) => {
    assert.equal(config.method, 'get')
    assert.ok(config.url)
    reads.push(config.url)
    assert.ok(config.url?.startsWith('/core/v1/subscriptions/admin/records?'))
    const userId = Number(
      new URL(config.url, 'https://synthetic.example').searchParams.get(
        'user_id'
      )
    )
    assert.ok([52, 53].includes(userId))
    return {
      config,
      data: userId === 52 ? await old : page([record(53, 502)]),
      headers: {},
      status: 200,
      statusText: 'OK',
    }
  }
  const drawer = (userId: number) => (
    <UserSubscriptionsDialog
      open
      onOpenChange={() => undefined}
      user={{ id: userId }}
    />
  )
  const view = await mount(drawer(52))
  try {
    await view.render(drawer(53))
    assert.match(document.body.textContent ?? '', /synthetic-buyer-53/)
    await act(async () => {
      finishOld(page([record()]))
      await flush()
    })
    assert.match(document.body.textContent ?? '', /synthetic-buyer-53/)
    assert.doesNotMatch(document.body.textContent ?? '', /synthetic-buyer-52/)
    assert.equal(reads.length, 2)
    assert.ok(!document.body.textContent?.includes('Add subscription'))
  } finally {
    await view.close()
  }
})

test('session changes cannot dispatch the previous operator command and failed reads stay distinct from empty records', async () => {
  let posts = 0
  api.defaults.adapter = async (config) => {
    if (config.method !== 'get') {
      posts++
      throw new Error('unexpected mutation')
    }
    return {
      config,
      data: page([record()]),
      headers: {},
      status: 200,
      statusText: 'OK',
    }
  }
  const view = await mount(<NativeSubscriptionRecords scope={scope} />)
  try {
    await click('Invalidate')
    await input('#subscription-invalidation-reason', 'Synthetic review')
    await act(async () => {
      login(100, 2, 'replacement-session')
      await flush()
    })
    await click('Confirm')
    assert.equal(posts, 0)
    assert.equal(dom.sessionStorage.getItem(recoveryKey), null)
  } finally {
    await view.close()
  }
  login()
  api.defaults.adapter = async () => {
    throw new Error('synthetic read failure')
  }
  const failed = await mount(<NativeSubscriptionRecords scope={scope} />)
  try {
    assert.match(
      document.body.textContent ?? '',
      /Failed to load subscription records/
    )
    assert.doesNotMatch(
      document.body.textContent ?? '',
      /No subscription records/
    )
  } finally {
    await failed.close()
  }
})
