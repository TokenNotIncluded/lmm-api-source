/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { Window } from 'happy-dom'

const dom = new Window({ url: 'https://console.example.test/' })
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLInputElement',
  'HTMLButtonElement',
  'customElements',
  'Node',
  'Element',
  'Event',
  'MouseEvent',
  'PointerEvent',
  'FocusEvent',
  'MutationObserver',
  'ResizeObserver',
  'requestAnimationFrame',
  'cancelAnimationFrame',
  'getComputedStyle',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: dom[key],
  })
}
Object.defineProperty(globalThis, 'matchMedia', {
  configurable: true,
  value: (media: string) => ({
    matches: false,
    media,
    onchange: null,
    addEventListener() {},
    removeEventListener() {},
    addListener() {},
    removeListener() {},
    dispatchEvent() {
      return false
    },
  }),
})
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
const { api } = await import('@/lib/api')
const { RedPackets } = await import('./index')

test('create button opens the actual dialog outside the layout slots', async () => {
  const i18n = createInstance()
  await i18n
    .use(initReactI18next)
    .init({ lng: 'en', resources: { en: { translation: {} } } })
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  const originalGet = api.get
  api.get = (async (url: string) => ({
    data: {
      success: true,
      data:
        url === '/api/red-packet/admin'
          ? []
          : url.startsWith('/api/redemption/?')
            ? {
                items: [
                  {
                    id: 1,
                    name: 'Current quota code',
                    reward_type: 'quota',
                    status: 1,
                    quota: 200,
                  },
                  { id: 2, name: 'Imported quota code', status: 1, quota: 100 },
                  {
                    id: 3,
                    name: 'Retired reset inventory',
                    reward_type: 'reset_voucher',
                    status: 1,
                    quota: 0,
                  },
                ],
              }
            : { items: [] },
    },
  })) as typeof api.get
  try {
    await act(async () => {
      root.render(
        <I18nextProvider i18n={i18n}>
          <QueryClientProvider client={client}>
            <RedPackets />
          </QueryClientProvider>
        </I18nextProvider>
      )
      await new Promise((resolve) => setTimeout(resolve, 0))
    })
    const button = [...container.querySelectorAll('button')].find((element) =>
      element.textContent?.includes('Create red packet')
    )
    assert.ok(button)
    assert.equal(document.querySelector('[role="dialog"]'), null)
    await act(async () => {
      button.click()
      await new Promise((resolve) => setTimeout(resolve, 20))
    })
    const dialog = document.querySelector('[role="dialog"]')
    assert.ok(
      dialog,
      'clicking Create must render a dialog, not only update hidden state'
    )
    assert.match(dialog.textContent ?? '', /Choose existing codes/)
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 20))
    })
    assert.match(dialog.textContent ?? '', /Current quota code/)
    assert.match(dialog.textContent ?? '', /Imported quota code/)
    assert.doesNotMatch(dialog.textContent ?? '', /Retired reset inventory/)
    assert.ok(dialog.querySelector('input'))
  } finally {
    await act(async () => root.unmount())
    api.get = originalGet
    client.clear()
    container.remove()
    await dom.happyDOM.abort()
  }
})

test('old reset prizes remain readable without offering their code or wallet redemption', async () => {
  const { renderToStaticMarkup } = await import('react-dom/server')
  const { RedPacketRewardCard } = await import('./public')
  const i18n = createInstance()
  await i18n
    .use(initReactI18next)
    .init({ lng: 'en', resources: { en: { translation: {} } } })
  const html = renderToStaticMarkup(
    <I18nextProvider i18n={i18n}>
      <RedPacketRewardCard
        index={0}
        reward={{
          claim_id: 1,
          claimed_at: 100,
          item_type: 'redemption',
          reward_type: 'reset_voucher',
          reset_plan_id: 42,
          name: 'Old reset prize',
          code: 'retired-synthetic-code',
        }}
      />
    </I18nextProvider>
  )
  assert.match(html, /Banked reset voucher for plan #42/)
  assert.match(html, /Read-only/)
  assert.doesNotMatch(
    html,
    /retired-synthetic-code|Redeem this code|Added to your wallet|<button/
  )
})
