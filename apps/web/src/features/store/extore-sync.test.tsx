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

afterEach(async () => {
  await act(async () => root?.unmount())
  root = undefined
  client?.clear()
  client = undefined
  storeApi.extoreConnections = originalConnections
  storeApi.extoreConnectionCatalog = originalCatalog
  storeApi.extoreSync = originalSync
  useAuthStore.getState().auth.setUser(null)
  document.body.replaceChildren()
})
after(() => dom.happyDOM.abort())

test('Extore connections render sync controls for imported products', async () => {
  useAuthStore.getState().auth.setUser({ id: 7, username: 'seller' } as never)
  storeApi.extoreConnections = async () => ({
    connections: [{ id: 'c1', shop_name: 'Demo shop', expires_at: 1 }],
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
          variants: [{ id: 'basic', name: 'Basic', enabled: true, attributes: {} }],
          card_summary: { total: 3, states: { available: 3 } },
        },
      ],
    }) as never
  await mount(<StoreExtoreConnections />)
  assert.match(document.body.textContent || '', /Demo shop/)
  assert.match(document.body.textContent || '', /Notes/)
  assert.match(document.body.textContent || '', /Extore code status/)
  await mount(<StoreExtoreProductSync productId='local-1' />)
  assert.match(document.body.textContent || '', /Imported from Extore/)
})
