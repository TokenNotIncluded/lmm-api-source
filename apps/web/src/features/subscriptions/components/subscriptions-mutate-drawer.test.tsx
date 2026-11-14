// @ts-expect-error Bun's test module is available only in the test runtime.
import { mock } from 'bun:test'
/* Copyright (C) 2026 LIghtJUNction; AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { Window } from 'happy-dom'

import type { PlanRecord } from '../types'

const dom = new Window({ url: 'https://console.example.test/subscriptions' })
dom.document.write('<!doctype html><html><head></head><body></body></html>')
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLInputElement',
  'HTMLSelectElement',
  'HTMLButtonElement',
  'HTMLFormElement',
  'SVGElement',
  'Node',
  'Element',
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
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: dom[key],
  })
}
Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
  configurable: true,
  value: true,
})
let refreshes = 0
mock.module('./subscriptions-provider', () => ({
  useSubscriptions: () => ({
    triggerRefresh: () => {
      refreshes++
    },
  }),
}))
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
function login(id = 71, sid = 'drawer-session-A') {
  useAuthStore.getState().auth.setBundle({
    user: {
      id,
      role: 100,
      username: 'synthetic-drawer-admin',
      wallet_unit: 'core_credit',
    },
    access_token: 'synthetic-drawer-token',
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
const { SubscriptionsMutateDrawer } =
  await import('./subscriptions-mutate-drawer')
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
const { toast } = await import('sonner')
const originalSuccess = toast.success
const originalError = toast.error
const notifications: string[] = []
const originalAdapter = api.defaults.adapter
const originalGet = api.get
const roots: ReturnType<typeof createRoot>[] = []
afterEach(async () => {
  await act(async () => {
    for (const root of roots.splice(0)) root.unmount()
  })
  api.defaults.adapter = originalAdapter
  api.get = originalGet
  toast.success = originalSuccess
  toast.error = originalError
  document.body.replaceChildren()
})
after(() => dom.close())
function required<T>(value: T | undefined): T {
  assert.ok(value !== undefined)
  return value
}
function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => {
    resolve = done
  })
  return { promise, resolve }
}
const flush = () => new Promise<void>((resolve) => setTimeout(resolve, 0))
function row(id: number): PlanRecord {
  return {
    plan: {
      id,
      title: `Plan ${id}`,
      price_amount: id,
      currency: 'USD',
      duration_unit: 'month',
      duration_value: 1,
      quota_reset_period: 'never',
      enabled: true,
      sort_order: 0,
      allow_balance_pay: true,
      allow_wallet_overflow: true,
      max_purchase_per_user: 0,
      total_amount: 0,
      waffo_pancake_products: [
        {
          product_type: 'subscription',
          product_id: `product-${id}`,
          enabled: true,
        },
      ],
    },
    payment_methods: ['balance', 'waffo_pancake', 'alipay'],
  }
}
function catalog(id: number, name: string) {
  return {
    message: 'success',
    data: {
      store_id: 'synthetic-store',
      products: [
        {
          id: `product-${id}`,
          name,
          status: 'active',
          product_type: 'subscription',
        },
      ],
    },
  }
}
function network(save?: ReturnType<typeof deferred<{ success: boolean }>>) {
  login()
  refreshes = 0
  notifications.length = 0
  toast.success = (message) => {
    notifications.push(String(message))
    return 'synthetic-toast'
  }
  toast.error = (message) => {
    notifications.push(String(message))
    return 'synthetic-toast'
  }
  // Deliver independent reads in either order while retaining the real HTTP
  // authentication/scope interceptors. Coalescing is not under test here.
  api.get = (url, config) =>
    originalGet(url, { ...config, disableDuplicate: true })
  const catalogs: ReturnType<typeof deferred<ReturnType<typeof catalog>>>[] = []
  const writes: { url?: string; data: unknown }[] = []
  api.defaults.adapter = async (config) => {
    let data: unknown
    if (config.url === '/api/group') data = { success: true, data: ['default'] }
    else if (
      config.url === '/api/option/waffo-pancake/subscription-product-options'
    ) {
      const pending = deferred<ReturnType<typeof catalog>>()
      catalogs.push(pending)
      data = await pending.promise
    } else if (config.method === 'put' && save) {
      writes.push({ url: config.url, data: config.data })
      data = await save.promise
    } else throw new Error(`Unexpected request ${config.method} ${config.url}`)
    return { config, data, headers: {}, status: 200, statusText: 'OK' }
  }
  return { catalogs, writes }
}
async function mount() {
  const root = createRoot(
    document.body.appendChild(document.createElement('div'))
  )
  roots.push(root)
  const closes: boolean[] = []
  async function render(currentRow?: PlanRecord, open = true) {
    await act(async () => {
      root.render(
        <I18nextProvider i18n={i18n}>
          <SubscriptionsMutateDrawer
            open={open}
            currentRow={currentRow}
            onOpenChange={(value) => {
              closes.push(value)
            }}
          />
        </I18nextProvider>
      )
      await flush()
    })
  }
  return { render, closes }
}
function title() {
  const input = document.querySelector<HTMLInputElement>('input[name="title"]')
  assert.ok(input)
  return input
}
function selectedProduct() {
  const select = document.querySelector(
    '[aria-label="Subscription — automatic renewal — Select a product"]'
  )
  assert.ok(select)
  return select.textContent
}
async function dirtyTitle() {
  await act(async () => {
    const setter = Object.getOwnPropertyDescriptor(
      dom.HTMLInputElement.prototype,
      'value'
    )?.set
    assert.ok(setter)
    setter.call(title(), 'Unsaved draft')
    title().dispatchEvent(new Event('input', { bubbles: true }))
    await flush()
  })
  assert.equal(title().value, 'Unsaved draft')
}

test('row A to B initializes B form and saved provider draft independently', async () => {
  network()
  const ui = await mount()
  await ui.render(row(1))
  await dirtyTitle()
  await ui.render(row(2))
  assert.equal(title().value, 'Plan 2')
  assert.equal(selectedProduct(), 'product-2')
  assert.equal(
    document.querySelector<HTMLInputElement>('#pancake-enable-subscription')
      ?.checked,
    true
  )
  assert.equal(
    document.querySelector<HTMLInputElement>('#pancake-enable-one_time')
      ?.checked,
    false
  )
})

test('close then reopen same row discards unsaved form and provider draft', async () => {
  network()
  const ui = await mount()
  await ui.render(row(1))
  await dirtyTitle()
  await act(async () => {
    document
      .querySelector<HTMLButtonElement>('#pancake-enable-subscription')
      ?.click()
    await flush()
  })
  assert.equal(
    document.querySelector<HTMLInputElement>('#pancake-enable-subscription')
      ?.checked,
    false
  )
  await ui.render(row(1), false)
  assert.equal(document.querySelector('input[name="title"]'), null)
  await ui.render(row(1))
  assert.equal(title().value, 'Plan 1')
  assert.equal(selectedProduct(), 'product-1')
  assert.equal(
    document.querySelector<HTMLInputElement>('#pancake-enable-subscription')
      ?.checked,
    true
  )
})

for (const transition of ['row switch', 'close/reopen'] as const) {
  test(`late catalog from old session cannot replace current catalog after ${transition}`, async () => {
    const net = network()
    const ui = await mount()
    await ui.render(row(1))
    if (transition === 'close/reopen') await ui.render(row(1), false)
    const id = transition === 'row switch' ? 2 : 1
    await ui.render(row(id))
    assert.equal(net.catalogs.length, 2)
    await act(async () => {
      required(net.catalogs[1]).resolve(catalog(id, 'Current catalog'))
      await flush()
    })
    assert.match(selectedProduct() || '', /Current catalog/)
    await act(async () => {
      required(net.catalogs[0]).resolve(catalog(id, 'Obsolete catalog'))
      await flush()
    })
    assert.match(selectedProduct() || '', /Current catalog/)
    assert.doesNotMatch(selectedProduct() || '', /Obsolete/)
    assert.equal(title().value, `Plan ${id}`)
  })
}

test('late successful save for A cannot close newly opened B', async () => {
  const pending = deferred<{ success: boolean }>()
  const net = network(pending)
  const ui = await mount()
  refreshes = 0
  await ui.render(row(1))
  await act(async () => {
    document
      .querySelector<HTMLFormElement>('#subscription-form')
      ?.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
    await flush()
  })
  assert.equal(
    net.writes.length,
    1,
    'real form validation must reach original A save'
  )
  assert.equal(required(net.writes[0]).url, '/api/subscription/admin/plans/1')
  assert.equal(
    JSON.parse(String(required(net.writes[0]).data)).plan.title,
    'Plan 1'
  )
  await ui.render(row(2))
  await act(async () => {
    pending.resolve({ success: true })
    await flush()
  })
  assert.equal(title().value, 'Plan 2')
  assert.deepEqual(ui.closes, [], 'A completion must not close B')
  assert.equal(refreshes, 0)
  assert.deepEqual(notifications, [])
})

for (const changed of ['account', 'session'] as const) {
  test(`provider result after ${changed} change cannot start another product or plan write`, async () => {
    network()
    const pending = deferred<{
      message: string
      data: { product_id: string; product_type: string }
    }>()
    const delegate = api.defaults.adapter
    assert.ok(typeof delegate === 'function')
    const writes: string[] = []
    api.defaults.adapter = async (config) => {
      if (config.method === 'post' || config.method === 'put') {
        writes.push(config.url || '')
        assert.equal(
          config.url,
          '/api/option/waffo-pancake/subscription-product'
        )
        assert.deepEqual(config.authScope, {
          userId: 71,
          sessionId: 'drawer-session-A',
        })
        const body = JSON.parse(String(config.data))
        assert.equal(body.product_type, 'one_time')
        assert.equal(body.name, 'Plan 1')
        const data = await pending.promise
        return { config, data, headers: {}, status: 200, statusText: 'OK' }
      }
      return delegate(config)
    }
    const ui = await mount()
    const source = row(1)
    source.plan.waffo_pancake_products = []
    await ui.render(source)
    await act(async () => {
      const enableBoth = Array.from(document.querySelectorAll('button')).find(
        (button) =>
          button.textContent?.trim() === 'Enable both purchase options'
      )
      assert.ok(enableBoth)
      enableBoth.click()
      await flush()
      document
        .querySelector<HTMLFormElement>('#subscription-form')
        ?.dispatchEvent(
          new Event('submit', { bubbles: true, cancelable: true })
        )
      await flush()
    })
    assert.deepEqual(writes, ['/api/option/waffo-pancake/subscription-product'])
    await act(async () => {
      login(changed === 'account' ? 72 : 71, 'drawer-session-B')
      await flush()
      pending.resolve({
        message: 'success',
        data: {
          product_id: 'actual-synthetic-created-id',
          product_type: 'one_time',
        },
      })
      await flush()
      await flush()
    })
    assert.deepEqual(
      writes,
      ['/api/option/waffo-pancake/subscription-product'],
      'no second provider or plan request after identity changes'
    )
    assert.deepEqual(ui.closes, [])
    assert.equal(refreshes, 0)
    assert.deepEqual(notifications, [])
  })
}

test('late manual catalog refresh cannot replace reopened session catalog', async () => {
  const net = network()
  const ui = await mount()
  await ui.render(row(1))
  await act(async () => {
    required(net.catalogs[0]).resolve(catalog(1, 'Initial catalog'))
    await flush()
  })
  await act(async () => {
    const refresh = Array.from(document.querySelectorAll('button')).find(
      (button) => button.textContent?.trim() === 'Refresh'
    )
    assert.ok(refresh)
    refresh.click()
    await flush()
  })
  assert.equal(net.catalogs.length, 2)
  await ui.render(row(1), false)
  await ui.render(row(1))
  assert.equal(net.catalogs.length, 3)
  await act(async () => {
    required(net.catalogs[2]).resolve(catalog(1, 'Reopened catalog'))
    await flush()
  })
  assert.match(selectedProduct() || '', /Reopened catalog/)
  await act(async () => {
    required(net.catalogs[1]).resolve(catalog(1, 'Obsolete manual refresh'))
    await flush()
  })
  assert.match(selectedProduct() || '', /Reopened catalog/)
  assert.deepEqual(notifications, [])
})

test('new-plan session initializes defaults independently of prior edit', async () => {
  network()
  const ui = await mount()
  await ui.render(row(1))
  await dirtyTitle()
  await ui.render(undefined)
  assert.equal(title().value, '')
  assert.equal(
    document.querySelector<HTMLInputElement>('input[name="price_amount"]')
      ?.value,
    '0'
  )
  assert.equal(
    document.querySelector<HTMLInputElement>('#pancake-enable-subscription')
      ?.checked,
    false
  )
  assert.equal(
    document.querySelector<HTMLInputElement>('#pancake-enable-one_time')
      ?.checked,
    false
  )
  assert.doesNotMatch(document.body.textContent || '', /product-1/)
})
