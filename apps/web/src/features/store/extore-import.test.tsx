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
const { StoreExtoreImport } = await import('./extore-import')
const { storeApi } = await import('./api')
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
let root: ReturnType<typeof createRoot> | undefined
let client: InstanceType<typeof QueryClient> | undefined
const originalCatalog = storeApi.extoreCatalog
const flush = () => new Promise((resolve) => setTimeout(resolve, 30))
async function mount(node: React.ReactNode) {
  const host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  const currentRoot = root,
    currentClient = client
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
  storeApi.extoreCatalog = originalCatalog
  document.body.replaceChildren()
})
after(() => dom.happyDOM.abort())

test('Extore callback shows per-source sync results', async () => {
  storeApi.extoreCatalog = async () => ({
    connection: { id: 'connection-1' },
    results: [
      {
        source_id: 'notes',
        product_id: 'local-1',
        success: true,
        message: 'Product synchronized from Extore',
        last_synced_at: 1,
      },
    ],
  })
  let imported = false
  await mount(
    <StoreExtoreImport
      callbackUrl='https://shop.example.test/store/manage?state=test&code=test&iss=https%3A%2F%2Fextore.lmm.best'
      onClose={() => undefined}
      onImported={() => {
        imported = true
      }}
    />
  )
  assert.match(document.body.textContent || '', /notes/)
  assert.match(
    document.body.textContent || '',
    /Product synchronized from Extore/
  )
  assert.equal(imported, true)
})
