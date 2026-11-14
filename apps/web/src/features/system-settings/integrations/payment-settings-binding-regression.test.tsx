/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
// @ts-expect-error Bun's test module is available only in the test runtime.
import { mock } from 'bun:test'
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

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
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: dom[key],
  })
}
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
const React = await import('react')
const { act } = React
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider, notifyManager } =
  await import('@tanstack/react-query')
notifyManager.setScheduler(queueMicrotask)
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { SettingsPageProvider } =
  await import('@/features/system-settings/components/settings-page-context')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { useSystemConfigStore, DEFAULT_CURRENCY_CONFIG } =
  await import('@/stores/system-config-store')
const i18n = createInstance()
await i18n.use(initReactI18next).init({
  lng: 'en',
  fallbackLng: 'en',
  resources: { en: { translation: {} }, fr: { translation: {} } },
})
function actor(id = 41, sid = 'session-A', role = 100) {
  const old = useAuthStore.getState().auth
  useAuthStore.setState({
    auth: {
      ...old,
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
    },
  })
}
function identity() {
  const { auth } = useAuthStore.getState()
  return {
    actor: auth.user?.id,
    role: auth.user?.role,
    sid: auth.session?.sid,
    language: i18n.language,
  }
}
async function flush() {
  await act(async () => {
    await new Promise<void>((resolve) => setImmediate(resolve))
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
  client.setQueryData(['system-options'], {
    success: true,
    message: '',
    data: [],
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
function button(text: string, scope: ParentNode = document) {
  const found = [...scope.querySelectorAll<HTMLButtonElement>('button')].find(
    (b) => b.textContent?.trim() === text
  )
  assert.ok(found, `button missing: ${text}`)
  return found
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
for (const method of ['get', 'post', 'put', 'delete'] as const) {
  api[method] = async (path: string) => {
    unexpected.push(`${method} ${path}`)
    throw new Error(`Uncontrolled API: ${method} ${path}`)
  }
}
afterEach(async () => {
  for (const { root, client } of mounts.splice(0)) {
    await act(async () => root.unmount())
    client.clear()
  }
  document.body.replaceChildren()
  assert.deepEqual(unexpected.splice(0), [])
})
after(() => dom.close())

// Observe committed child props while retaining the actual child UI and callbacks.
const actualPancake =
  await import('@/features/system-settings/integrations/waffo-pancake-settings-section')
const ActualPancake = actualPancake.WaffoPancakeSettingsSection
type PancakeProps = React.ComponentProps<typeof ActualPancake>
type BindingSnapshot = {
  selected: PancakeProps['selectedBinding']
  saved: PancakeProps['savedBinding']
  values: PancakeProps['values']
}
let pancakeSnapshot: BindingSnapshot | undefined
function bindingSnapshot() {
  assert.ok(pancakeSnapshot)
  return pancakeSnapshot
}
mock.module(
  '@/features/system-settings/integrations/waffo-pancake-settings-section',
  () => ({
    WaffoPancakeSettingsSection: (props: PancakeProps) => {
      React.useEffect(() => {
        pancakeSnapshot = structuredClone({
          selected: props.selectedBinding,
          saved: props.savedBinding,
          values: props.values,
        })
      })
      return <ActualPancake {...props} />
    },
  })
)
const { PaymentSettingsSection } =
  await import('@/features/system-settings/integrations/payment-settings-section')
const methods = (name: string) => [
  { name, icon: '', payMethodType: 'CARD', payMethodName: name },
]
function props(source = 'A') {
  return {
    defaultValues: {
      PayAddress: '',
      EpayId: '',
      EpayKey: '',
      MinTopUp: 1,
      CustomCallbackAddress: '',
      PayMethods: '[]',
      AmountOptions: '[1,2]',
      AmountDiscount: '{}',
      StripeApiSecret: '',
      StripeWebhookSecret: '',
      StripePriceId: '',
      StripeUnitPrice: 1,
      StripeMinTopUp: 1,
      StripePromotionCodesEnabled: false,
      CreemApiKey: '',
      CreemWebhookSecret: '',
      CreemTestMode: false,
      CreemProducts: '[]',
    },
    waffoDefaultValues: {
      WaffoEnabled: true,
      WaffoApiKey: '',
      WaffoPrivateKey: '',
      WaffoPublicCert: '',
      WaffoSandboxPublicCert: '',
      WaffoSandboxApiKey: '',
      WaffoSandboxPrivateKey: '',
      WaffoSandbox: false,
      WaffoMerchantId: '',
      WaffoCurrency: 'USD',
      WaffoUnitPrice: 1,
      WaffoMinTopUp: 1,
      WaffoNotifyUrl: '',
      WaffoReturnUrl: '',
      WaffoPayMethods: JSON.stringify(methods(source)),
    },
    waffoPancakeDefaultValues: {
      WaffoPancakeMerchantID: 'synthetic-merchant',
      WaffoPancakePrivateKey: '',
      WaffoPancakeReturnURL: `https://return.example.test/${source}`,
    },
    waffoPancakeProvisionedStoreID: `store-${source}`,
    waffoPancakeProvisionedProductID: `product-${source}`,
    complianceDefaults: {
      confirmed: true,
      termsVersion: 'v1',
      confirmedAt: 1,
      confirmedBy: 41,
    },
  }
}
function setup() {
  // This verifies draft synchronization, not root authorization or payment execution.
  actor(41, 'session-A', 1)
  useSystemConfigStore.getState().setConfig({
    currency: {
      ...DEFAULT_CURRENCY_CONFIG,
      creditsPerUsd: 500000,
      creditsPerUsdExact: '500000',
      quotaPerUnit: 500000,
      quotaDisplayType: 'USD',
    },
  })
}
test('Payment parent: provisioned binding A-B-A synchronizes selected AND saved; identical/default-only props retain user selection', async () => {
  setup()
  pancakeSnapshot = undefined
  const calls: Array<{
    path?: string
    body?: Record<string, string>
    merchant_id?: string
    private_key?: string
  }> = []
  api.post = (async (path: string, body: Record<string, string>) => {
    calls.push({ path, body, ...identity() })
    assert.equal(
      path,
      '/api/option/waffo-pancake/catalog',
      'no pair/save/payment requests authorized'
    )
    assert.deepEqual(body, { merchant_id: '', private_key: '' })
    return {
      data: {
        message: 'success',
        data: {
          stores: ['A', 'B', 'draft'].map((x) => ({
            id: `store-${x}`,
            name: `Store ${x}`,
            status: 'active',
            prodEnabled: true,
            onetimeProducts: [
              { id: `product-${x}`, name: `Product ${x}`, status: 'active' },
            ],
          })),
        },
      },
    }
  }) as typeof api.post
  const ui = await mount(<PaymentSettingsSection {...props()} />)
  await click(button('Waffo Pancake'))
  await until(
    () =>
      calls.length === 1 &&
      ui.host.querySelectorAll('[role="combobox"]').length === 2,
    'actual catalog supplies real select controls'
  )
  assert.deepEqual(bindingSnapshot().selected, {
    storeID: 'store-A',
    productID: 'product-A',
  })
  const selectOption = async (index: number, text: string) => {
    const controls =
      ui.host.querySelectorAll<HTMLButtonElement>('[role="combobox"]')
    await click(controls[index])
    await until(
      () =>
        [...document.querySelectorAll('[role="option"]')].some((o) =>
          o.textContent?.includes(text)
        ),
      `option ${text} rendered`
    )
    const option = [
      ...document.querySelectorAll<HTMLElement>('[role="option"]'),
    ].find((o) => o.textContent?.includes(text))
    assert.ok(option)
    await click(option)
  }
  await selectOption(0, 'Store draft')
  await selectOption(1, 'Product draft')
  const drafted = structuredClone(bindingSnapshot())
  assert.deepEqual(drafted.selected, {
    storeID: 'store-draft',
    productID: 'product-draft',
  })
  assert.deepEqual(drafted.saved, {
    storeID: 'store-A',
    productID: 'product-A',
  })
  const returnInput = ui.host.querySelector<HTMLInputElement>(
    'input[placeholder="https://example.com/wallet"]'
  )
  assert.ok(returnInput)
  await edit(returnInput, 'https://return.example.test/user-edit')
  await ui.render(<PaymentSettingsSection {...props()} />)
  assert.deepEqual(bindingSnapshot().selected, drafted.selected)
  assert.equal(
    bindingSnapshot().values.WaffoPancakeReturnURL,
    'https://return.example.test/user-edit'
  )
  const unrelated = props()
  unrelated.waffoDefaultValues.WaffoPayMethods = JSON.stringify(
    methods('other')
  )
  await ui.render(<PaymentSettingsSection {...unrelated} />)
  assert.deepEqual(
    bindingSnapshot().selected,
    drafted.selected,
    'defaults-only source change preserves user binding selection'
  )
  await ui.render(<PaymentSettingsSection {...props('B')} />)
  const afterB = structuredClone(bindingSnapshot())
  await ui.render(<PaymentSettingsSection {...props('A')} />)
  const afterA = structuredClone(bindingSnapshot())
  assert.deepEqual(
    [afterB.selected, afterA.selected],
    [
      { storeID: 'store-B', productID: 'product-B' },
      { storeID: 'store-A', productID: 'product-A' },
    ]
  )
  assert.deepEqual(
    [afterB.saved, afterA.saved],
    [afterB.selected, afterA.selected]
  )
  assert.deepEqual(
    [afterB.values.WaffoPancakeReturnURL, afterA.values.WaffoPancakeReturnURL],
    ['https://return.example.test/B', 'https://return.example.test/A']
  )
  assert.equal(
    calls.length,
    1,
    'default changes must not initiate additional catalog requests'
  )
})

test('Pancake store changes clear only the draft product; choosing its product never changes the saved binding', async () => {
  setup()
  pancakeSnapshot = undefined
  let catalogCalls = 0
  api.post = (async (path: string) => {
    assert.equal(path, '/api/option/waffo-pancake/catalog')
    catalogCalls++
    return {
      data: {
        message: 'success',
        data: {
          stores: ['A', 'B'].map((x) => ({
            id: `store-${x}`,
            name: `Store ${x}`,
            status: 'active',
            prodEnabled: true,
            onetimeProducts: [
              { id: `product-${x}`, name: `Product ${x}`, status: 'active' },
            ],
          })),
        },
      },
    }
  }) as typeof api.post
  const ui = await mount(<PaymentSettingsSection {...props('B')} />)
  await click(button('Waffo Pancake'))
  await until(
    () =>
      catalogCalls === 1 &&
      ui.host.querySelectorAll('[role="combobox"]').length === 2,
    'catalog loaded'
  )
  assert.deepEqual(bindingSnapshot().selected, {
    storeID: 'store-B',
    productID: 'product-B',
  })
  const selectOption = async (index: number, text: string) => {
    await click(
      ui.host.querySelectorAll<HTMLButtonElement>('[role="combobox"]')[index]
    )
    await until(
      () =>
        [...document.querySelectorAll('[role="option"]')].some((o) =>
          o.textContent?.includes(text)
        ),
      `option ${text}`
    )
    const option = [
      ...document.querySelectorAll<HTMLElement>('[role="option"]'),
    ].find((o) => o.textContent?.includes(text))
    assert.ok(option)
    await click(option)
  }
  await selectOption(0, 'Store A')
  assert.deepEqual(bindingSnapshot().selected, {
    storeID: 'store-A',
    productID: '',
  })
  assert.deepEqual(bindingSnapshot().saved, {
    storeID: 'store-B',
    productID: 'product-B',
  })
  await selectOption(1, 'Product A')
  assert.deepEqual(bindingSnapshot().selected, {
    storeID: 'store-A',
    productID: 'product-A',
  })
  assert.deepEqual(bindingSnapshot().saved, {
    storeID: 'store-B',
    productID: 'product-B',
  })
  assert.equal(catalogCalls, 1)
})

test('same store ID across merchant/catalog A-B keeps the provisioned B product; refreshing B catalog preserves a still-valid selection', async () => {
  setup()
  pancakeSnapshot = undefined
  const calls: Array<{
    path?: string
    body?: Record<string, string>
    merchant_id?: string
    private_key?: string
  }> = []
  const catalog = (products: string[]) => ({
    data: {
      message: 'success',
      data: {
        stores: [
          {
            id: 'shared-store',
            name: 'Shared Store',
            status: 'active',
            prodEnabled: true,
            onetimeProducts: products.map((id) => ({
              id,
              name: id,
              status: 'active',
            })),
          },
        ],
      },
    },
  })
  api.post = (async (path: string, body: Record<string, string>) => {
    assert.equal(
      path,
      '/api/option/waffo-pancake/catalog',
      'no save, provision or payment requests'
    )
    calls.push({ path, body })
    if (calls.length === 1) {
      return catalog(['merchant-A-product', 'shared-valid-product'])
    }
    assert.equal(body.merchant_id, 'merchant-B')
    if (body.private_key === 'synthetic-B-key') {
      return catalog(['merchant-B-product', 'shared-valid-product'])
    }
    assert.equal(body.private_key, 'synthetic-B-refreshed-key')
    return catalog([
      'new-neighbor-product',
      'merchant-B-product',
      'shared-valid-product',
    ])
  }) as typeof api.post
  const a = props('A')
  a.waffoPancakeDefaultValues.WaffoPancakeMerchantID = 'merchant-A'
  a.waffoPancakeProvisionedStoreID = 'shared-store'
  a.waffoPancakeProvisionedProductID = 'merchant-A-product'
  const ui = await mount(<PaymentSettingsSection {...a} />)
  await click(button('Waffo Pancake'))
  await until(
    () =>
      calls.length === 1 &&
      ui.host.querySelectorAll('[role="combobox"]').length === 2,
    'A catalog loaded'
  )
  assert.deepEqual(bindingSnapshot().selected, {
    storeID: 'shared-store',
    productID: 'merchant-A-product',
  })
  const b = props('B')
  b.waffoPancakeDefaultValues.WaffoPancakeMerchantID = 'merchant-B'
  b.waffoPancakeDefaultValues.WaffoPancakePrivateKey = 'synthetic-B-key'
  b.waffoPancakeProvisionedStoreID = 'shared-store'
  b.waffoPancakeProvisionedProductID = 'merchant-B-product'
  await ui.render(<PaymentSettingsSection {...b} />)
  assert.deepEqual(
    bindingSnapshot().selected,
    { storeID: 'shared-store', productID: 'merchant-B-product' },
    'provisioned IDs survive while the new catalog is pending'
  )
  assert.deepEqual(bindingSnapshot().saved, bindingSnapshot().selected)
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 850))
  })
  await until(
    () =>
      calls.length === 2 &&
      pancakeSnapshot?.selected.productID === 'merchant-B-product',
    'B catalog and product loaded'
  )
  assert.deepEqual(bindingSnapshot().selected, {
    storeID: 'shared-store',
    productID: 'merchant-B-product',
  })
  assert.match(
    ui.host.querySelectorAll('[role="combobox"]')[1]?.textContent ?? '',
    /merchant-B-product/
  )
  await click(
    ui.host.querySelectorAll<HTMLButtonElement>('[role="combobox"]')[1]
  )
  await until(
    () =>
      [...document.querySelectorAll('[role="option"]')].some((o) =>
        o.textContent?.includes('shared-valid-product')
      ),
    'valid unsaved product option'
  )
  const validDraftOption = [
    ...document.querySelectorAll<HTMLElement>('[role="option"]'),
  ].find((o) => o.textContent?.includes('shared-valid-product'))
  assert.ok(validDraftOption)
  await click(validDraftOption)
  assert.deepEqual(bindingSnapshot().selected, {
    storeID: 'shared-store',
    productID: 'shared-valid-product',
  })
  assert.deepEqual(bindingSnapshot().saved, {
    storeID: 'shared-store',
    productID: 'merchant-B-product',
  })
  const keyInput = ui.host.querySelector<HTMLTextAreaElement>(
    'textarea[autocomplete="new-password"]'
  )
  assert.ok(keyInput)
  await edit(keyInput, 'synthetic-B-refreshed-key')
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 850))
  })
  await until(
    () => calls.length === 3,
    'unchanged-store refreshed catalog returned'
  )
  assert.deepEqual(bindingSnapshot().selected, {
    storeID: 'shared-store',
    productID: 'shared-valid-product',
  })
  assert.deepEqual(bindingSnapshot().saved, {
    storeID: 'shared-store',
    productID: 'merchant-B-product',
  })
  assert.match(
    ui.host.querySelectorAll('[role="combobox"]')[1]?.textContent ?? '',
    /shared-valid-product/
  )
})

function sharedCatalog(productIDs: string[]) {
  return {
    data: {
      message: 'success',
      data: {
        stores: [
          {
            id: 'shared-store',
            name: 'Shared Store',
            status: 'active',
            prodEnabled: true,
            onetimeProducts: productIDs.map((id) => ({
              id,
              name: id,
              status: 'active',
            })),
          },
        ],
      },
    },
  }
}
function sharedProps(merchant: string, product: string) {
  const result = props(merchant)
  result.waffoPancakeDefaultValues.WaffoPancakeMerchantID = merchant
  result.waffoPancakeProvisionedStoreID = 'shared-store'
  result.waffoPancakeProvisionedProductID = product
  return result
}
async function chooseProduct(
  ui: Awaited<ReturnType<typeof mount>>,
  product: string
) {
  await click(
    ui.host.querySelectorAll<HTMLButtonElement>('[role="combobox"]')[1]
  )
  await until(
    () =>
      [...document.querySelectorAll('[role="option"]')].some((o) =>
        o.textContent?.includes(product)
      ),
    `product ${product}`
  )
  const option = [
    ...document.querySelectorAll<HTMLElement>('[role="option"]'),
  ].find((o) => o.textContent?.includes(product))
  assert.ok(option)
  await click(option)
}

test('merchant A response cannot overwrite B binding or catalog even before B debounce starts', async () => {
  setup()
  pancakeSnapshot = undefined
  let finishA: (value: ReturnType<typeof sharedCatalog>) => void = () => {
    throw new Error('A request absent')
  }
  const calls: Array<{
    path?: string
    body?: Record<string, string>
    merchant_id?: string
    private_key?: string
  }> = []
  api.post = (async (path: string, body: Record<string, string>) => {
    assert.equal(path, '/api/option/waffo-pancake/catalog')
    calls.push(body)
    if (calls.length === 1) {
      return new Promise<ReturnType<typeof sharedCatalog>>((resolve) => {
        finishA = resolve
      })
    }
    assert.equal(body.merchant_id, 'merchant-B')
    return sharedCatalog(['product-B'])
  }) as typeof api.post
  const ui = await mount(
    <PaymentSettingsSection {...sharedProps('merchant-A', 'product-A')} />
  )
  await click(button('Waffo Pancake'))
  await until(() => calls.length === 1, 'A catalog request pending')
  const b = sharedProps('merchant-B', 'product-B')
  b.waffoPancakeDefaultValues.WaffoPancakePrivateKey = 'synthetic-B-key'
  await ui.render(<PaymentSettingsSection {...b} />)
  assert.equal(calls.length, 1)
  await act(async () => finishA(sharedCatalog(['product-A'])))
  assert.deepEqual(bindingSnapshot().selected, {
    storeID: 'shared-store',
    productID: 'product-B',
  })
  assert.deepEqual(bindingSnapshot().saved, bindingSnapshot().selected)
  assert.doesNotMatch(ui.host.textContent ?? '', /product-A/)
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 850))
  })
  await until(
    () =>
      calls.length === 2 &&
      ui.host.querySelectorAll('[role="combobox"]').length === 2,
    'B catalog arrives'
  )
  assert.deepEqual(bindingSnapshot().selected, {
    storeID: 'shared-store',
    productID: 'product-B',
  })
  assert.match(
    ui.host.querySelectorAll('[role="combobox"]')[1]?.textContent ?? '',
    /product-B/
  )
})

test('failed catalog refresh preserves a valid unsaved selection and saved binding', async () => {
  setup()
  pancakeSnapshot = undefined
  let calls = 0
  api.post = (async (path: string) => {
    assert.equal(path, '/api/option/waffo-pancake/catalog')
    if (++calls === 1) return sharedCatalog(['saved-product', 'operator-draft'])
    throw new Error('synthetic catalog unavailable')
  }) as typeof api.post
  const ui = await mount(
    <PaymentSettingsSection {...sharedProps('merchant-B', 'saved-product')} />
  )
  await click(button('Waffo Pancake'))
  await until(
    () =>
      calls === 1 && ui.host.querySelectorAll('[role="combobox"]').length === 2,
    'initial catalog'
  )
  await chooseProduct(ui, 'operator-draft')
  const key = ui.host.querySelector<HTMLTextAreaElement>(
    'textarea[autocomplete="new-password"]'
  )
  assert.ok(key)
  await edit(key, 'synthetic-refresh-key')
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 850))
  })
  await until(() => calls === 2, 'failed refresh completed')
  assert.deepEqual(bindingSnapshot().selected, {
    storeID: 'shared-store',
    productID: 'operator-draft',
  })
  assert.deepEqual(bindingSnapshot().saved, {
    storeID: 'shared-store',
    productID: 'saved-product',
  })
  assert.match(
    ui.host.querySelectorAll('[role="combobox"]')[1]?.textContent ?? '',
    /operator-draft/
  )
})

test('invalid selection falls back to the latest saved product, not the request-start saved product or first catalog item', async () => {
  setup()
  pancakeSnapshot = undefined
  let calls = 0
  let finishRefresh: (value: ReturnType<typeof sharedCatalog>) => void = () => {
    throw new Error('refresh absent')
  }
  api.post = (async (path: string) => {
    assert.equal(path, '/api/option/waffo-pancake/catalog')
    if (++calls === 1) {
      return sharedCatalog(['old-saved', 'obsolete-draft', 'latest-saved'])
    }
    return new Promise<ReturnType<typeof sharedCatalog>>((resolve) => {
      finishRefresh = resolve
    })
  }) as typeof api.post
  const initial = sharedProps('merchant-B', 'old-saved')
  const ui = await mount(<PaymentSettingsSection {...initial} />)
  await click(button('Waffo Pancake'))
  await until(
    () =>
      calls === 1 && ui.host.querySelectorAll('[role="combobox"]').length === 2,
    'initial catalog'
  )
  await chooseProduct(ui, 'obsolete-draft')
  const key = ui.host.querySelector<HTMLTextAreaElement>(
    'textarea[autocomplete="new-password"]'
  )
  assert.ok(key)
  await edit(key, 'synthetic-refresh-key')
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 850))
  })
  await until(() => calls === 2, 'catalog refresh pending')
  await ui.render(
    <PaymentSettingsSection {...sharedProps('merchant-B', 'latest-saved')} />
  )
  await chooseProduct(ui, 'obsolete-draft')
  assert.deepEqual(bindingSnapshot().saved, {
    storeID: 'shared-store',
    productID: 'latest-saved',
  })
  await act(async () =>
    finishRefresh(sharedCatalog(['first-neighbor', 'latest-saved']))
  )
  assert.deepEqual(bindingSnapshot().selected, {
    storeID: 'shared-store',
    productID: 'latest-saved',
  })
  assert.deepEqual(bindingSnapshot().saved, bindingSnapshot().selected)
  assert.equal(calls, 2)
  assert.match(
    ui.host.querySelectorAll('[role="combobox"]')[1]?.textContent ?? '',
    /latest-saved/
  )
})

for (const field of ['merchant', 'key'] as const) {
  test(`verification phase releases when ${field} is cleared and restored to the deduplicated A config`, async () => {
    setup()
    pancakeSnapshot = undefined
    let calls = 0
    let finishA: (value: ReturnType<typeof sharedCatalog>) => void = () => {
      throw new Error('A request absent')
    }
    api.post = (async (path: string) => {
      assert.equal(path, '/api/option/waffo-pancake/catalog')
      if (++calls === 1) return sharedCatalog(['product-A'])
      return new Promise<ReturnType<typeof sharedCatalog>>((resolve) => {
        finishA = resolve
      })
    }) as typeof api.post
    const initial = sharedProps('merchant-A', 'product-A')
    const ui = await mount(<PaymentSettingsSection {...initial} />)
    await click(button('Waffo Pancake'))
    await until(
      () =>
        calls === 1 &&
        ui.host.querySelectorAll('[role="combobox"]').length === 2,
      'saved catalog loaded'
    )
    const privateKey = ui.host.querySelector<HTMLTextAreaElement>(
      'textarea[autocomplete="new-password"]'
    )
    assert.ok(privateKey)
    await edit(privateKey, 'synthetic-A-key')
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 850))
    })
    await until(() => calls === 2, 'typed A verification pending')
    const create = [
      ...ui.host.querySelectorAll<HTMLButtonElement>('button'),
    ].find((item) => item.textContent?.includes('+ Create'))
    assert.ok(create)
    assert.equal(create.disabled, true)
    assert.match(
      ui.host.textContent ?? '',
      /Verifying credentials and pulling stores/
    )
    const input =
      field === 'merchant'
        ? ui.host.querySelector<HTMLInputElement>(
            'input[placeholder="MER_xxx"]'
          )
        : ui.host.querySelector<HTMLTextAreaElement>(
            'textarea[autocomplete="new-password"]'
          )
    assert.ok(input)
    await edit(input, '')
    assert.doesNotMatch(
      ui.host.textContent ?? '',
      /Verifying credentials and pulling stores/
    )
    assert.equal(
      create.disabled,
      field === 'merchant',
      'blank key uses saved credentials, while blank merchant remains incomplete'
    )
    await edit(input, field === 'merchant' ? 'merchant-A' : 'synthetic-A-key')
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 850))
    })
    assert.equal(
      calls,
      2,
      'restoring the same last signature must not invent a catalog request'
    )
    assert.deepEqual(
      {
        disabled: create.disabled,
        verifying: (ui.host.textContent ?? '').includes(
          'Verifying credentials and pulling stores'
        ),
      },
      { disabled: false, verifying: false }
    )
    await act(async () => finishA(sharedCatalog(['product-A'])))
    assert.equal(create.disabled, false)
    assert.doesNotMatch(
      ui.host.textContent ?? '',
      /Verifying credentials and pulling stores/
    )
    assert.equal(calls, 2)
  })
}

test('verification phase belongs to active B; a stale A completion cannot clear B spinner or enable pair creation', async () => {
  setup()
  pancakeSnapshot = undefined
  let calls = 0
  let finishA: (value: ReturnType<typeof sharedCatalog>) => void = () => {
    throw new Error('A request absent')
  }
  let finishB: (value: ReturnType<typeof sharedCatalog>) => void = () => {
    throw new Error('B request absent')
  }
  api.post = (async (path: string) => {
    assert.equal(path, '/api/option/waffo-pancake/catalog')
    return new Promise<ReturnType<typeof sharedCatalog>>((resolve) => {
      if (++calls === 1) finishA = resolve
      else finishB = resolve
    })
  }) as typeof api.post
  const a = sharedProps('merchant-A', 'product-A')
  a.waffoPancakeDefaultValues.WaffoPancakePrivateKey = 'synthetic-A-key'
  const ui = await mount(<PaymentSettingsSection {...a} />)
  await click(button('Waffo Pancake'))
  await until(() => calls === 1, 'A verification pending')
  const b = sharedProps('merchant-B', 'product-B')
  b.waffoPancakeDefaultValues.WaffoPancakePrivateKey = 'synthetic-B-key'
  await ui.render(<PaymentSettingsSection {...b} />)
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 850))
  })
  await until(() => calls === 2, 'B verification pending')
  const create = [
    ...ui.host.querySelectorAll<HTMLButtonElement>('button'),
  ].find((item) => item.textContent?.includes('+ Create'))
  assert.ok(create)
  await act(async () => finishA(sharedCatalog(['product-A'])))
  assert.deepEqual(
    {
      disabled: create.disabled,
      verifying: (ui.host.textContent ?? '').includes(
        'Verifying credentials and pulling stores'
      ),
    },
    { disabled: true, verifying: true }
  )
  assert.deepEqual(bindingSnapshot().selected, {
    storeID: 'shared-store',
    productID: 'product-B',
  })
  await act(async () => finishB(sharedCatalog(['product-B'])))
  assert.deepEqual(
    {
      disabled: create.disabled,
      verifying: (ui.host.textContent ?? '').includes(
        'Verifying credentials and pulling stores'
      ),
    },
    { disabled: false, verifying: false }
  )
  assert.deepEqual(bindingSnapshot().selected, {
    storeID: 'shared-store',
    productID: 'product-B',
  })
})
