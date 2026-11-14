/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, beforeEach, test } from 'node:test'

import { Window } from 'happy-dom'

const unexpected: string[] = []
const reject = ({ request }: { request: { url: string } }): never => {
  unexpected.push(request.url)
  throw new Error(`Uncontrolled network: ${request.url}`)
}
const dom = new Window({
  url: 'https://console.example.test/settings',
  settings: {
    fetch: {
      interceptor: {
        beforeAsyncRequest: async (c) => reject(c),
        beforeSyncRequest: reject,
      },
    },
  },
})
dom.document.write('<!doctype html><html><head></head><body></body></html>')
Object.defineProperty(dom.document, 'compatMode', {
  configurable: true,
  value: 'CSS1Compat',
})
const originalGlobals = new Map<string, PropertyDescriptor | undefined>()
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLInputElement',
  'HTMLTextAreaElement',
  'HTMLButtonElement',
  'HTMLLabelElement',
  'HTMLFormElement',
  'history',
  'location',
  'scrollTo',
  'File',
  'FileReader',
  'FormData',
  'SVGElement',
  'Node',
  'NodeFilter',
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
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
const React = await import('react')
const { act } = React
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider, notifyManager } =
  await import('@tanstack/react-query')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { SettingsPageProvider } =
  await import('@/features/system-settings/components/settings-page-context')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { toast } = await import('sonner')
const messages: { kind: string; message: string }[] = []
const originalToasts = new Map(
  ['error', 'success', 'info', 'warning'].map((kind) => [
    kind,
    Reflect.get(toast, kind),
  ])
)
for (const kind of ['error', 'success', 'info', 'warning'] as const) {
  Reflect.set(toast, kind, (message: unknown) => {
    messages.push({ kind, message: String(message) })
    return 0
  })
}
const i18n = createInstance()
await i18n.use(initReactI18next).init({
  lng: 'en',
  fallbackLng: 'en',
  resources: { en: { translation: {} }, fr: { translation: {} } },
})
function actor(id = 41, sid = 'session-A', role = 100) {
  useAuthStore.getState().auth.setBundle({
    access_token: `synthetic-${id}-${sid}`,
    token_type: 'Bearer',
    access_expires_at: 2000000000,
    user: { id, username: `synthetic-${id}`, role },
    session: {
      sid,
      current: true,
      login_method: 'password',
      ip: '',
      user_agent: '',
      created_at: 1,
      last_active_at: 1,
      expires_at: 2000000000,
    },
  })
}
async function flush() {
  await act(async () => {
    await new Promise<void>((resolve) => notifyManager.schedule(resolve))
  })
}
async function until(check: () => boolean, label: string) {
  for (let i = 0; i < 80 && !check(); i++) await flush()
  assert.ok(check(), `${label}; DOM=${document.body.textContent?.slice(-1800)}`)
}
const mounts: {
  root: ReturnType<typeof createRoot>
  client: InstanceType<typeof QueryClient>
}[] = []
async function mount(element: React.ReactNode) {
  const host = document.createElement('div'),
    actions = document.createElement('div')
  document.body.append(host, actions)
  const root = createRoot(host)
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: Infinity },
      mutations: { retry: false },
    },
  })
  mounts.push({ root, client })
  const render = async (node: React.ReactNode) => {
    await act(async () =>
      root.render(
        <QueryClientProvider client={client}>
          <I18nextProvider i18n={i18n}>
            <SettingsPageProvider actionsContainer={actions}>
              {node}
            </SettingsPageProvider>
          </I18nextProvider>
        </QueryClientProvider>
      )
    )
    await flush()
  }
  await render(element)
  return { host, actions, client, render }
}
async function click(element: HTMLElement) {
  await act(async () => element.click())
  await flush()
}
async function edit(
  input: HTMLInputElement | HTMLTextAreaElement,
  value: string
) {
  assert.ok(input)
  const proto =
    input.tagName === 'TEXTAREA'
      ? dom.HTMLTextAreaElement.prototype
      : dom.HTMLInputElement.prototype
  const setter = Object.getOwnPropertyDescriptor(proto, 'value')?.set
  assert.ok(setter)
  await act(async () => {
    setter.call(input, value)
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
  await flush()
}
afterEach(async () => {
  for (const { root, client } of mounts.splice(0)) {
    await act(async () => root.unmount())
    client.clear()
  }
  document.body.replaceChildren()
  assert.deepEqual(unexpected.splice(0), [])
  messages.splice(0)
})
after(() => {
  for (const [kind, method] of originalToasts) Reflect.set(toast, kind, method)
  dom.close()
  for (const [key, descriptor] of originalGlobals) {
    if (descriptor) Object.defineProperty(globalThis, key, descriptor)
    else Reflect.deleteProperty(globalThis, key)
  }
})

const { StoreCheckout } = await import('./product-page')
const { installSupportedAuthBrowser } =
  await import('@/test-support/supported-auth-browser')
installSupportedAuthBrowser()
// The shared auth fixture implements request(name, options, callback).
// The checkout journal uses the standard omitted-options overload; retain the
// same queued/abort implementation for both forms in this browser context.
const locks = navigator.locks
const lockRequestDescriptor = Object.getOwnPropertyDescriptor(locks, 'request')
Object.defineProperty(locks, 'request', {
  configurable: true,
  value: new Proxy(locks.request, {
    apply(target, receiver, args) {
      return Reflect.apply(
        target,
        receiver,
        args.length === 2 ? [args[0], {}, args[1]] : args
      )
    },
  }),
})
after(() => {
  if (lockRequestDescriptor) {
    Object.defineProperty(locks, 'request', lockRequestDescriptor)
  } else Reflect.deleteProperty(locks, 'request')
})
const originalAdapter = api.defaults.adapter
type Product = React.ComponentProps<typeof StoreCheckout>['product']
const product: Product = {
  id: 'product-fixture',
  seller_id: 9,
  title: 'Fixture',
  description: 'Text',
  image_urls: [],
  contact: '',
  links: [],
  price_quota: 500000,
  template: 'card-key',
  delivery_strategy: 'sequential',
  payment_methods: ['balance'],
  pickup_login_required: false,
  pickup_code_required: false,
  email_pickup_link: false,
  status: 'published',
  official: true,
  available_stock: 5,
  promotion_expires_at: 0,
  created_at: 1,
  updated_at: 1,
  review_note: '',
  default_variant_id: 'fixture-default',
  variants: [
    {
      id: 'fixture-default',
      product_id: 'product-fixture',
      name: 'Default',
      price_quota: 500000,
      template: 'card-key',
      enabled: true,
      is_default: true,
      created_at: 1,
      updated_at: 1,
      inventory_total: 5,
      inventory_available: 5,
      reserved_stock: 0,
      sale_available: 5,
      trading_paused: false,
    },
  ],
}
type RequestRecord = {
  method: string
  path: string
  body: Record<string, unknown>
}
let requests: RequestRecord[]
let transport: (request: RequestRecord) => Promise<unknown>
beforeEach(() => {
  actor()
  localStorage.clear()
  sessionStorage.clear()
  requests = []
  transport = async (request) => {
    if (request.method === 'get' && request.path === '/api/store/config') {
      return {
        success: true,
        data: { store_access_supported: false, store_terms_supported: false },
      }
    }
    if (request.method === 'get' && request.path === '/api/store/disclaimer') {
      return {
        success: true,
        data: { version: 'v1', text: 'Synthetic disclaimer', accepted: false },
      }
    }
    unexpected.push(`${request.method} ${request.path}`)
    throw new Error('Unexpected product transport')
  }
  api.defaults.adapter = async (config) => {
    const request = {
      method: config.method ?? '',
      path: config.url ?? '',
      body:
        typeof config.data === 'string'
          ? JSON.parse(config.data)
          : (config.data ?? {}),
    }
    requests.push(request)
    return {
      config,
      data: await transport(request),
      status: 200,
      statusText: 'fixture',
      headers: {},
    }
  }
})
afterEach(() => {
  api.defaults.adapter = originalAdapter
})
const writes = () => requests.filter(({ method }) => method !== 'get')
function input(id: string) {
  const node = document.getElementById(id) as HTMLInputElement | null
  assert.ok(node)
  return node
}
function button(label: string) {
  const node = [...document.querySelectorAll<HTMLButtonElement>('button')].find(
    (item) => item.textContent?.trim() === label
  )
  assert.ok(node, label)
  return node
}
async function ready() {
  await until(
    () =>
      document.getElementById('store-pickup-code') !== null &&
      !button('Place order').disabled,
    'checkout ready'
  )
}

test('checkout keeps same-product drafts on stock refresh while clamping quantity to the new authoritative capacity', async () => {
  const ui = await mount(<StoreCheckout product={product} />)
  await ready()
  await edit(input('store-pickup-code'), 'private-code')
  await edit(input('store-pickup-email'), 'buyer@example.test')
  await edit(input('store-quantity'), '4')
  await ui.render(
    <StoreCheckout
      product={{
        ...product,
        available_stock: 2,
        variants: [{ ...product.variants![0], sale_available: 2 }],
      }}
    />
  )
  assert.equal(input('store-quantity').value, '2')
  assert.equal(input('store-pickup-code').value, 'private-code')
  assert.equal(input('store-pickup-email').value, 'buyer@example.test')
  await ui.render(
    <StoreCheckout
      product={{
        ...product,
        available_stock: 5,
        variants: [{ ...product.variants![0], sale_available: 5 }],
      }}
    />
  )
  assert.equal(input('store-quantity').value, '2')
  assert.equal(writes().length, 0)
})

test('quantity editing retains an incomplete draft until the capacity source actually changes', async () => {
  const ui = await mount(<StoreCheckout product={product} />)
  await ready()
  await edit(input('store-quantity'), '')
  assert.equal(input('store-quantity').value, '')
  assert.equal(button('Place order').disabled, true)
  await ui.render(<StoreCheckout product={{ ...product }} />)
  assert.equal(input('store-quantity').value, '')
  await ui.render(
    <StoreCheckout
      product={{
        ...product,
        available_stock: 2,
        variants: [{ ...product.variants![0], sale_available: 2 }],
      }}
    />
  )
  assert.equal(input('store-quantity').value, '1')
  assert.equal(writes().length, 0)
})

for (const boundary of ['product', 'seller', 'session'] as const) {
  test(`checkout ${boundary} ownership change starts a fresh draft without an order or payment request`, async () => {
    const ui = await mount(<StoreCheckout product={product} />)
    await ready()
    await edit(input('store-pickup-code'), 'private-code')
    await edit(input('store-pickup-email'), 'buyer@example.test')
    await edit(input('store-quantity'), '3')
    if (boundary === 'session') await act(async () => actor(41, 'session-B'))
    else {
      await ui.render(
        <StoreCheckout
          product={{
            ...product,
            ...(boundary === 'product'
              ? {
                  id: 'next-product',
                  variants: product.variants!.map((variant) => ({
                    ...variant,
                    product_id: 'next-product',
                  })),
                }
              : { seller_id: 10 }),
          }}
        />
      )
    }
    await ready()
    assert.equal(input('store-pickup-code').value, '')
    assert.equal(input('store-pickup-email').value, '')
    assert.equal(input('store-quantity').value, '1')
    assert.equal(writes().length, 0)
  })
}

test('a new disclaimer version revokes consent while equal-version refresh preserves it and cancel creates nothing', async () => {
  const ui = await mount(
    <StoreCheckout product={{ ...product, official: false }} />
  )
  await ready()
  await click(button('Place order'))
  const consent = document.getElementById('store-platform-disclaimer-accepted')
  assert.ok(consent)
  await click(consent)
  assert.equal(button('Agree and place order').disabled, false)
  await act(async () =>
    ui.client.setQueryData(['store', 'disclaimer', 41], {
      version: 'v1',
      text: 'Updated text same version',
      accepted: false,
    })
  )
  await flush()
  assert.equal(button('Agree and place order').disabled, false)
  await act(async () =>
    ui.client.setQueryData(['store', 'disclaimer', 41], {
      version: 'v2',
      text: 'New version',
      accepted: false,
    })
  )
  await flush()
  assert.equal(button('Agree and place order').disabled, true)
  const liveConsent = document.querySelector<HTMLElement>('[role="checkbox"]')
  assert.ok(liveConsent)
  assert.equal(liveConsent.getAttribute('aria-checked'), 'false')
  await click(button('Close'))
  assert.equal(document.querySelector('[role="dialog"]'), null)
  assert.equal(writes().length, 0)
})

test('native read-only product never mounts legacy checkout or dispatches order/payment writes', async () => {
  await mount(<StoreCheckout product={{ ...product, record_kind: 'native' }} />)
  assert.equal(button('Buy now').disabled, true)
  assert.equal(document.getElementById('store-pickup-code'), null)
  assert.equal(requests.length, 0)
})

const { useStoreCheckoutRecovery, isStoreCheckoutActorCurrent } =
  await import('./checkout-recovery')
const { createStoreCheckoutIntentJournal } = await import('./checkout-intent')
const { STORE_ACCESS_COPY } = await import('./access-copy')
const guestSession = {
  guest_id: '22222222-2222-4222-8222-222222222222',
  token: 'A'.repeat(43),
  expires_at: 2000000000,
}
const recoveredOrder: import('./types').StoreOrder = {
  id: 'recovered-order',
  trade_no: 'recovered-private-trade',
  buyer_id: 0,
  seller_id: 9,
  product_id: product.id,
  product_title: product.title,
  quantity: 1,
  unit_price_quota: 500000,
  price_quota: 500000,
  fee_quota: 0,
  payment_method: 'external:epay',
  status: 'pending',
  amount_minor: 100,
  currency: 'USD',
  frozen_usd_fx: '1',
  payment_issued: false,
  pickup_login_required: false,
  pickup_code_required: false,
  email_pickup_link: false,
  created_at: 1,
  paid_at: 0,
  expires_at: 2000000000,
}
async function prepareGuestRecovery() {
  useAuthStore.getState().auth.setUser(null)
  sessionStorage.setItem(
    'lmm.store.guest-session.v1',
    JSON.stringify(guestSession)
  )
  const journal = createStoreCheckoutIntentJournal({
    isActorCurrent: isStoreCheckoutActorCurrent,
  })
  return journal.prepare(
    { kind: 'guest', guestId: guestSession.guest_id },
    { productId: product.id, quantity: 1 },
    { product_id: product.id, quantity: 1, payment_method: 'external:epay' }
  )
}
function recoveryRecord() {
  const data = JSON.parse(
    localStorage.getItem('lmm:store:checkout-intents') || '{"records":[]}'
  )
  return data.records[0] as
    | { state: string; orderId?: string; requestKey: string }
    | undefined
}
function RecoveryHarness({
  onAccepted,
}: {
  onAccepted: NonNullable<
    Parameters<typeof useStoreCheckoutRecovery>[0]['onOrderAccepted']
  >
}) {
  const value = useStoreCheckoutRecovery({
    productId: product.id,
    supported: true,
    termsSupported: false,
    guest: guestSession,
    onOrderAccepted: onAccepted,
  })
  return <div>{value.order?.trade_no}</div>
}
function pendingResponse() {
  let resolve!: (value: unknown) => void
  const promise = new Promise<unknown>((accept) => {
    resolve = accept
  })
  return { promise, resolve }
}

test('accepted guest recovery publishes its callback only after the original journal records the pending order, without creating or paying', async () => {
  const prepared = await prepareGuestRecovery()
  transport = async (request) => {
    assert.equal(request.path, '/api/store/guest/orders/lookup')
    assert.equal(request.body.request_key, prepared.record.requestKey)
    return { success: true, data: recoveredOrder }
  }
  const delivered: unknown[] = []
  const onAccepted = (
    orderId: string,
    actor: import('./checkout-intent').StoreCheckoutActor
  ) => {
    assert.equal(recoveryRecord()?.state, 'pending')
    assert.equal(recoveryRecord()?.orderId, recoveredOrder.id)
    delivered.push({ orderId, actor })
  }
  const ui = await mount(<RecoveryHarness onAccepted={onAccepted} />)
  await until(() => delivered.length === 1, 'accepted receipt callback')
  await ui.render(<RecoveryHarness onAccepted={onAccepted} />)
  assert.deepEqual(delivered, [
    {
      orderId: recoveredOrder.id,
      actor: { kind: 'guest', guestId: guestSession.guest_id },
    },
  ])
  assert.equal(recoveryRecord()?.requestKey, prepared.record.requestKey)
  assert.deepEqual(
    writes().map(({ path }) => path),
    ['/api/store/guest/orders/lookup']
  )
})

for (const boundary of ['unmount', 'account', 'session'] as const) {
  test(`accepted receipt callback cannot cross an ${boundary} boundary while lookup is pending`, async () => {
    await prepareGuestRecovery()
    const pending = pendingResponse()
    transport = async (request) => {
      assert.equal(request.path, '/api/store/guest/orders/lookup')
      return pending.promise
    }
    let deliveries = 0
    const ui = await mount(
      <RecoveryHarness
        onAccepted={() => {
          deliveries++
        }}
      />
    )
    await until(() => requests.length === 1, 'original lookup pending')
    if (boundary === 'unmount') {
      const mounted = mounts.pop()
      assert.ok(mounted)
      await act(async () => mounted.root.unmount())
      mounted.client.clear()
    } else {
      await act(async () => {
        actor(41, 'session-B')
        if (boundary === 'session') useAuthStore.getState().auth.setUser(null)
      })
    }
    await act(async () =>
      pending.resolve({ success: true, data: recoveredOrder })
    )
    await flush()
    await flush()
    assert.equal(deliveries, 0)
    assert.doesNotMatch(ui.host.textContent ?? '', /recovered-private-trade/)
    assert.deepEqual(
      writes().map(({ path }) => path),
      ['/api/store/guest/orders/lookup']
    )
  })
}

test('a recovered guest order remains visible and journalled pending when its receipt index write fails, with the original error feedback', async () => {
  const prepared = await prepareGuestRecovery()
  const guestProduct: Product = {
    ...product,
    visibility: 'public',
    purchase_login_required: false,
    payment_methods: ['external:epay'],
  }
  transport = async (request) => {
    if (request.path === '/api/store/config') {
      return {
        success: true,
        data: { store_access_supported: true, store_terms_supported: false },
      }
    }
    if (request.path === `/api/store/products/${product.id}`) {
      return { success: true, data: guestProduct }
    }
    if (request.path === '/api/store/guest/disclaimer') {
      return {
        success: true,
        data: { version: 'v1', text: 'Synthetic disclaimer', accepted: true },
      }
    }
    assert.equal(request.path, '/api/store/guest/orders/lookup')
    assert.equal(request.body.request_key, prepared.record.requestKey)
    return { success: true, data: recoveredOrder }
  }
  const key = 'lmm.store.guest-order-ids.v1'
  const descriptor = Object.getOwnPropertyDescriptor(sessionStorage, 'setItem')
  const original = sessionStorage.setItem
  Object.defineProperty(sessionStorage, 'setItem', {
    configurable: true,
    value: (name: string, value: string) => {
      if (name === key) {
        throw new Error('synthetic receipt-index storage failure')
      }
      original.call(sessionStorage, name, value)
    },
  })
  try {
    assert.throws(
      () => sessionStorage.setItem(key, '{}'),
      /synthetic receipt-index/
    )
    const ui = await mount(<StoreCheckout product={guestProduct} />)
    await until(
      () => ui.host.textContent?.includes(recoveredOrder.trade_no) === true,
      'recovered order visible'
    )
    assert.ok(
      ui.host.textContent?.includes(STORE_ACCESS_COPY.orderReceiptFailed)
    )
    assert.equal(recoveryRecord()?.state, 'pending')
    assert.equal(recoveryRecord()?.orderId, recoveredOrder.id)
    assert.equal(recoveryRecord()?.requestKey, prepared.record.requestKey)
    assert.deepEqual(
      writes().map(({ path }) => path),
      ['/api/store/guest/orders/lookup']
    )
  } finally {
    if (descriptor) Object.defineProperty(sessionStorage, 'setItem', descriptor)
    else Reflect.deleteProperty(sessionStorage, 'setItem')
  }
})

type RecoverySnapshot = {
  loading: boolean
  orderId: string | null
  requestKey: string | null
  error: unknown
}
function RecoveryInitializationHarness({
  productId = product.id,
  guest = guestSession as typeof guestSession | null,
  supported = true,
  observe,
}: {
  productId?: string
  guest?: typeof guestSession | null
  supported?: boolean
  observe: (snapshot: RecoverySnapshot) => void
}) {
  const value = useStoreCheckoutRecovery({
    productId,
    guest,
    supported,
    termsSupported: false,
  })
  React.useEffect(() => {
    observe({
      loading: value.loading,
      orderId: value.order?.id ?? null,
      requestKey: value.record?.requestKey ?? null,
      error: value.error,
    })
  })
  return <div>{value.order?.trade_no}</div>
}

test('recovery without an actor commits idle immediately and performs no transport', async () => {
  const snapshots: RecoverySnapshot[] = []
  const observe = (snapshot: RecoverySnapshot) => snapshots.push(snapshot)
  const ui = await mount(
    <RecoveryInitializationHarness guest={null} observe={observe} />
  )
  assert.ok(snapshots.length)
  assert.deepEqual(snapshots[0], {
    loading: false,
    orderId: null,
    requestKey: null,
    error: null,
  })
  await ui.render(
    <RecoveryInitializationHarness guest={null} observe={observe} />
  )
  assert.ok(snapshots.every((snapshot) => !snapshot.loading))
  assert.deepEqual(requests, [])
})

test('recovery product A to B to A clears committed state before lookup and preserves the pending journal', async () => {
  const prepared = await prepareGuestRecovery()
  transport = async (request) => {
    assert.equal(request.path, '/api/store/guest/orders/lookup')
    assert.equal(request.body.request_key, prepared.record.requestKey)
    return { success: true, data: recoveredOrder }
  }
  const snapshots: RecoverySnapshot[] = []
  const observe = (snapshot: RecoverySnapshot) => snapshots.push(snapshot)
  const ui = await mount(<RecoveryInitializationHarness observe={observe} />)
  await until(
    () => snapshots.at(-1)?.orderId === recoveredOrder.id,
    'A recovery'
  )
  const beforeB = snapshots.length
  await ui.render(
    <RecoveryInitializationHarness
      productId='other-product'
      observe={observe}
    />
  )
  assert.deepEqual(snapshots[beforeB], {
    loading: true,
    orderId: null,
    requestKey: null,
    error: null,
  })
  await until(() => snapshots.at(-1)?.loading === false, 'B idle')
  assert.equal(snapshots.at(-1)?.orderId, null)
  assert.equal(recoveryRecord()?.state, 'pending')
  assert.equal(recoveryRecord()?.requestKey, prepared.record.requestKey)
  const beforeA = snapshots.length
  await ui.render(<RecoveryInitializationHarness observe={observe} />)
  assert.deepEqual(snapshots[beforeA], {
    loading: true,
    orderId: null,
    requestKey: null,
    error: null,
  })
  await until(
    () => snapshots.at(-1)?.orderId === recoveredOrder.id,
    'A recovered again'
  )
  assert.deepEqual(
    writes().map(({ path }) => path),
    ['/api/store/guest/orders/lookup', '/api/store/guest/orders/lookup']
  )
  assert.equal(recoveryRecord()?.state, 'pending')
})

test('initialization clears a transport error at the source boundary without discarding UNKNOWN or enabling unsupported replay', async () => {
  const prepared = await prepareGuestRecovery()
  transport = async (request) => {
    assert.equal(request.path, '/api/store/guest/orders/lookup')
    assert.equal(request.body.request_key, prepared.record.requestKey)
    throw new Error('synthetic lookup transport failure')
  }
  const snapshots: RecoverySnapshot[] = []
  const observe = (snapshot: RecoverySnapshot) => snapshots.push(snapshot)
  const ui = await mount(<RecoveryInitializationHarness observe={observe} />)
  await until(
    () => snapshots.at(-1)?.loading === false,
    'failed lookup finished'
  )
  assert.ok(snapshots.at(-1)?.error)
  assert.equal(recoveryRecord()?.state, 'unknown')
  const boundary = snapshots.length
  await ui.render(
    <RecoveryInitializationHarness supported={false} observe={observe} />
  )
  assert.deepEqual(snapshots[boundary], {
    loading: true,
    orderId: null,
    requestKey: null,
    error: null,
  })
  await until(
    () => snapshots.at(-1)?.loading === false,
    'unsupported recovery finished'
  )
  const issue = snapshots.at(-1)?.error
  assert.ok(issue instanceof Error)
  assert.equal(issue.message, STORE_ACCESS_COPY.orderUnknown)
  assert.equal(recoveryRecord()?.state, 'unknown')
  assert.equal(recoveryRecord()?.requestKey, prepared.record.requestKey)
  assert.equal(requests.length, 1)
  transport = async (request) => {
    assert.equal(request.path, '/api/store/guest/orders/lookup')
    assert.equal(request.body.request_key, prepared.record.requestKey)
    return { success: true, data: recoveredOrder }
  }
  await ui.render(<RecoveryInitializationHarness observe={observe} />)
  await until(
    () => snapshots.at(-1)?.orderId === recoveredOrder.id,
    'supported recovery restored'
  )
  assert.equal(snapshots.at(-1)?.error, null)
  assert.equal(recoveryRecord()?.state, 'pending')
  assert.deepEqual(
    writes().map(({ path }) => path),
    ['/api/store/guest/orders/lookup', '/api/store/guest/orders/lookup']
  )
})
