/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
// @ts-expect-error Bun's test module is available only in the test runtime.
import { mock } from 'bun:test'
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { Window } from 'happy-dom'

const dom = new Window({ url: 'http://localhost/' })
for (const [key, value] of Object.entries({
  window: dom,
  document: dom.document,
  navigator: dom.navigator,
  HTMLElement: dom.HTMLElement,
  Element: dom.Element,
  Node: dom.Node,
  MutationObserver: dom.MutationObserver,
  getComputedStyle: dom.getComputedStyle,
  localStorage: dom.localStorage,
  sessionStorage: dom.sessionStorage,
  requestAnimationFrame: dom.requestAnimationFrame,
  cancelAnimationFrame: dom.cancelAnimationFrame,
  IS_REACT_ACT_ENVIRONMENT: true,
})) {
  Object.defineProperty(globalThis, key, { configurable: true, value })
}

const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
mock.module('@/components/page-transition', () => ({
  FadeIn: ({ children }: { children: import('react').ReactNode }) => children,
}))
mock.module('@visactor/react-vchart', () => ({
  VChart: () => null,
}))
const { api } = await import('@/lib/api')
const { UserCharts } = await import('./user-charts')

const originalGet = api.get
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
let dispose: (() => Promise<void>) | undefined

async function renderCharts() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  await act(async () => {
    root.render(
      <QueryClientProvider client={client}>
        <I18nextProvider i18n={i18n}>
          <UserCharts
            filters={{
              timeGranularity: 'day',
              selectedRange: 7,
              topUserLimit: 10,
            }}
            onFiltersChange={() => {}}
          />
        </I18nextProvider>
      </QueryClientProvider>
    )
    await new Promise((resolve) => setTimeout(resolve, 0))
  })
  dispose = async () => {
    await act(async () => root.unmount())
    client.clear()
  }
  return container
}

afterEach(async () => {
  await dispose?.()
  dispose = undefined
  api.get = originalGet
  document.body.replaceChildren()
})
after(() => dom.close())

test('user statistics exposes incomplete native-only coverage', async () => {
  api.get = (async () => ({
    data: {
      success: true,
      availability: 'partial',
      coverage: {
        scope: 'administrator_all_accounts',
        native_requests: true,
        historical_imports: false,
        complete: false,
      },
      data: [],
    },
  })) as typeof api.get

  const container = await renderCharts()
  assert.match(
    container.querySelector('[role="status"]')?.textContent ?? '',
    /Partial usage across all accounts: only native Core requests are included; imported historical usage is unavailable\./
  )
  assert.equal(container.querySelector('[role="alert"]'), null)
})

test('failed user statistics can be retried without becoming an empty result', async () => {
  let calls = 0
  api.get = (async () => {
    calls += 1
    if (calls === 1) throw new Error('HTTP 503')
    return { data: { success: true, data: [] } }
  }) as typeof api.get

  const container = await renderCharts()
  assert.match(container.textContent ?? '', /Usage could not be loaded/)
  assert.equal(container.querySelector('[role="alert"]') !== null, true)
  const retry = [...container.querySelectorAll('button')].find(
    (button) => button.textContent === 'Retry'
  )
  assert.ok(retry)
  await act(async () => {
    retry.click()
    await new Promise((resolve) => setTimeout(resolve, 0))
  })
  assert.equal(calls, 2)
  assert.equal(container.querySelector('[role="alert"]'), null)
})
