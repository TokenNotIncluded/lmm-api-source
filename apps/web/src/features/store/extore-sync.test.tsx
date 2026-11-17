/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { Window } from 'happy-dom'
import type React from 'react'

const dom = new Window({ url: 'https://shop.example.test/store/manage' })
dom.document.write('<!doctype html><html><body></body></html>')
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
  'CSSStyleSheet',
  'localStorage',
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

const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { StoreExtoreConnections, StoreExtoreProductSync } =
  await import('./extore-sync')
const { storeApi } = await import('./api')
const { useAuthStore } = await import('@/stores/auth-store')

const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })

let root: ReturnType<typeof createRoot> | undefined
let client: InstanceType<typeof QueryClient> | undefined
const originalConnections = storeApi.extoreConnections
const originalCatalog = storeApi.extoreConnectionCatalog
const originalSync = storeApi.extoreSync
const originalSyncAll = storeApi.extoreSyncAll
const originalDisconnect = storeApi.extoreDisconnect
const flush = () => new Promise((resolve) => setTimeout(resolve, 30))

async function mount(node: React.ReactNode) {
  const host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  const currentRoot = root
  const currentClient = client
  await act(async () => {
    currentRoot.render(
      <QueryClientProvider client={currentClient}>
        <I18nextProvider i18n={i18n}>{node}</I18nextProvider>
      </QueryClientProvider>
    )
    await flush()
  })
  await act(flush)
}

async function click(label: string) {
  const button = [
    ...document.querySelectorAll<HTMLButtonElement>('button'),
  ].find((element) => element.textContent?.trim() === label)
  assert.ok(button, label)
  await act(async () => {
    button.click()
    await flush()
  })
  await act(flush)
}

afterEach(async () => {
  await act(async () => root?.unmount())
  root = undefined
  client?.clear()
  client = undefined
  storeApi.extoreConnections = originalConnections
  storeApi.extoreConnectionCatalog = originalCatalog
  storeApi.extoreSync = originalSync
  storeApi.extoreSyncAll = originalSyncAll
  storeApi.extoreDisconnect = originalDisconnect
  useAuthStore.getState().auth.setUser(null)
  document.body.replaceChildren()
})
after(() => dom.happyDOM.abort())

test('Extore connections render sync controls for imported products', async () => {
  useAuthStore.getState().auth.setUser({ id: 7, username: 'seller' } as never)
  storeApi.extoreConnections = async () => ({
    connections: [
      {
        id: 'c1',
        origin: 'https://extore.lmm.best',
        shop_name: 'Demo shop',
        expires_at: Math.floor(Date.now() / 1000) + 3600,
      },
    ],
    sources: [
      {
        connection_id: 'c1',
        source_id: 'notes',
        product_id: 'local-1',
        last_synced_at: 1700000000,
      },
    ],
  })
  storeApi.extoreConnectionCatalog = async () =>
    ({
      schema: 'extore.commerce-catalog.v1',
      issuer: 'https://extore.lmm.best',
      shop: { id: 'shop', name: 'Demo shop' },
      grant_id: 'grant',
      products: [
        {
          schema: 'extore.product-listing.v1',
          id: 'notes',
          shop_id: 'shop',
          revision: 'a'.repeat(64),
          redemption_url: 'https://extore.lmm.best/',
          semantics: {
            price: 'reference',
            inventory: 'not_exported',
            payment: 'external_sales_platform',
            redemption: 'extore',
          },
          product: { name: 'Notes' },
          variants: [
            { id: 'basic', name: 'Basic', enabled: true, attributes: {} },
          ],
          card_summary: { total: 3, states: { available: 3 } },
        },
      ],
    }) as never
  await mount(<StoreExtoreConnections />)
  assert.equal(document.body.textContent?.includes('Notes'), false)
  await click('View products')
  const deadline = Date.now() + 3000
  while (
    !document.body.textContent?.includes('Notes') &&
    Date.now() < deadline
  ) {
    await act(flush)
  }
  assert.match(document.body.textContent || '', /Demo shop/)
  assert.match(document.body.textContent || '', /Notes/)
  assert.match(document.body.textContent || '', /Extore code status/)
  await act(async () => root?.unmount())
  root = undefined
  client?.clear()
  await mount(<StoreExtoreProductSync productId='local-1' />)
  assert.match(document.body.textContent || '', /Imported from Extore/)
})

test('expired connections offer reconnection without fetching or syncing', async () => {
  useAuthStore.getState().auth.setUser({ id: 7, username: 'seller' } as never)
  storeApi.extoreConnections = async () => ({
    connections: [
      {
        id: 'expired',
        shop_name: 'Expired shop',
        expires_at: 1,
        status: 'expired',
      },
    ],
    sources: [],
  })
  let requests = 0
  let reconnects = 0
  storeApi.extoreConnectionCatalog = async () => {
    requests++
    throw new Error('must not fetch')
  }
  await mount(
    <StoreExtoreConnections
      onConnect={() => {
        reconnects++
      }}
    />
  )
  assert.match(document.body.textContent || '', /Expired/)
  assert.equal(
    [...document.querySelectorAll('button')].some(
      (button) => button.textContent === 'Sync all products'
    ),
    false
  )
  await click('Connect again')
  assert.equal(reconnects, 1)
  assert.equal(requests, 0)
})

test('batch sync reports individual failures and links successful imports', async () => {
  useAuthStore.getState().auth.setUser({ id: 7, username: 'seller' } as never)
  storeApi.extoreConnections = async () => ({
    connections: [
      {
        id: 'c1',
        origin: 'https://extore.lmm.best',
        shop_name: 'Demo shop',
        expires_at: Math.floor(Date.now() / 1000) + 3600,
      },
    ],
    sources: [],
  })
  let synced = ''
  storeApi.extoreSyncAll = async (id) => {
    synced = id
    return {
      connection_id: id,
      results: [
        {
          source_id: 'ok',
          product_id: 'local-ok',
          success: true,
          message: 'Product synchronized from Extore',
        },
        {
          source_id: 'unpriced',
          success: false,
          message:
            'Configure the selling price for every variant in Extore before syncing',
        },
      ],
    }
  }
  await mount(<StoreExtoreConnections />)
  await click('Sync all products')
  assert.equal(synced, 'c1')
  assert.match(document.body.textContent || '', /1 synced · 1 need attention/)
  assert.match(document.body.textContent || '', /Configure the selling price/)
  assert.equal(
    document.querySelector('a[href="/store/preview/local-ok"]')?.textContent,
    'View in store'
  )
})

test('disconnect requires confirmation and keeps failed revocation recoverable', async () => {
  useAuthStore.getState().auth.setUser({ id: 7, username: 'seller' } as never)
  storeApi.extoreConnections = async () => ({
    connections: [
      {
        id: 'c1',
        shop_name: 'Demo shop',
        expires_at: Math.floor(Date.now() / 1000) + 3600,
      },
    ],
    sources: [],
  })
  let requests = 0
  storeApi.extoreDisconnect = async () => {
    requests++
    throw new Error('Upstream unavailable')
  }
  await mount(<StoreExtoreConnections />)
  await click('Disconnect')
  assert.equal(requests, 0)
  const dialog = document.querySelector('[role="alertdialog"]')
  assert.ok(dialog)
  const confirm = [
    ...dialog.querySelectorAll<HTMLButtonElement>('button'),
  ].find((button) => button.textContent === 'Disconnect')
  assert.ok(confirm)
  await act(async () => {
    confirm.click()
    await flush()
  })
  await act(flush)
  assert.equal(requests, 1)
  assert.match(document.body.textContent || '', /Upstream unavailable/)
  assert.ok(document.querySelector('[role="alertdialog"]'))
  assert.match(document.body.textContent || '', /Demo shop/)
})
