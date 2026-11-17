/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
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
  Event: dom.Event,
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
const { useAuthStore } = await import('@/stores/auth-store')
const { api } = await import('@/lib/api')
const { OverviewUsageCharts } = await import('./overview-usage-charts')
const { overviewUsageRange } = await import('./overview-personalization-data')
const originalUser = useAuthStore.getState().auth.user
const originalGet = api.get
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
let dispose: (() => Promise<void>) | undefined

afterEach(async () => {
  await dispose?.()
  dispose = undefined
  api.get = originalGet
  useAuthStore.getState().auth.setUser(originalUser)
  document.body.replaceChildren()
})
after(() => dom.close())

test('the actual all-image token chart renders unavailable instead of a fabricated zero line', async () => {
  useAuthStore
    .getState()
    .auth.setUser({ id: 881, username: 'image-owner', role: 1 })
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const range = overviewUsageRange(7)
  client.setQueryData(['overview-recorded-usage', 881, false, 7], {
    ...range,
    rows: [
      {
        created_at: range.end_timestamp,
        count: 1,
        quota: 2000,
        model_name: 'image-model',
        token_used: null,
        output_image_count: 2,
        token_count_scope: 'settled_text_requests',
      },
    ],
  })
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  act(() =>
    root.render(
      <QueryClientProvider client={client}>
        <I18nextProvider i18n={i18n}>
          <OverviewUsageCharts />
        </I18nextProvider>
      </QueryClientProvider>
    )
  )
  dispose = async () => {
    await act(async () => root.unmount())
    client.clear()
  }
  const tokensButton = [...container.querySelectorAll('button')].find(
    (button) => button.textContent === 'Tokens'
  )
  assert.ok(tokensButton)
  await act(async () => tokensButton.click())
  assert.match(
    container.querySelector('[role="status"]')?.textContent ?? '',
    /Unavailable/
  )
  assert.match(
    container.textContent ?? '',
    /Tokens cover settled text requests only\./
  )
  assert.match(container.textContent ?? '', /Images \(billing quantity\): 2/)
  assert.equal(container.querySelector('.overview-chart-grid'), null)
  assert.equal(container.querySelector('[role="alert"]'), null)
})

test('administrator usage requests all accounts and leaves uncovered history unavailable', async () => {
  useAuthStore
    .getState()
    .auth.setUser({ id: 882, username: 'administrator', role: 100 })
  const requests: string[] = []
  api.get = (async (url: string) => {
    requests.push(url)
    const allAccounts = url === '/api/data'
    return {
      data: {
        success: true,
        availability: allAccounts ? 'partial' : 'available',
        coverage: {
          scope: allAccounts
            ? 'administrator_all_accounts'
            : 'actor_personal_usage',
          native_requests: true,
          historical_imports: !allAccounts,
          complete: !allAccounts,
        },
        data: [],
      },
    }
  }) as typeof api.get
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  // A personal cache for the same identity must not satisfy an admin query.
  client.setQueryData(
    ['overview-recorded-usage', 882, false, 7],
    {
      ...overviewUsageRange(7),
      rows: [],
    },
    { updatedAt: 0 }
  )
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  dispose = async () => {
    await act(async () => root.unmount())
    client.clear()
  }
  await act(async () => {
    root.render(
      <QueryClientProvider client={client}>
        <I18nextProvider i18n={i18n}>
          <OverviewUsageCharts />
        </I18nextProvider>
      </QueryClientProvider>
    )
    await new Promise((resolve) => setTimeout(resolve, 20))
  })
  // React Query notifies observers on its next scheduled turn.
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 20))
  })
  assert.equal(requests.length, 1)
  assert.equal(requests[0], '/api/data')
  assert.match(container.textContent ?? '', /All accounts: native Core usage/)
  assert.match(
    container.textContent ?? '',
    /Imported history is available in Usage logs or My account/
  )
  assert.match(container.textContent ?? '', /No native requests in this period/)
  assert.equal(
    container.querySelector('a[href="/usage-logs/common"]')?.textContent,
    'Usage Logs'
  )
  assert.doesNotMatch(
    container.textContent ?? '',
    /Your account only|No recorded usage in this period|Your charts will appear/
  )
  assert.equal(container.querySelector('.overview-chart-grid'), null)
  assert.equal(container.querySelector('[role="alert"]'), null)

  const scope = container.querySelector<HTMLSelectElement>(
    'select[aria-label="Usage scope"]'
  )
  assert.ok(scope)
  await act(async () => {
    scope.value = 'personal'
    scope.dispatchEvent(new Event('change', { bubbles: true }))
    await new Promise((resolve) => setTimeout(resolve, 20))
  })
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 20))
  })
  assert.deepEqual(requests, ['/api/data', '/api/data/self'])
  assert.match(
    container.textContent ?? '',
    /Your account: current usage and imported history/
  )
  assert.match(container.textContent ?? '', /No recorded usage in this period/)
  assert.doesNotMatch(
    container.textContent ?? '',
    /No native requests in this period/
  )

  await act(async () => {
    useAuthStore
      .getState()
      .auth.setUser({ id: 882, username: 'administrator', role: 1 })
  })
  assert.match(container.textContent ?? '', /Your account only/)
  assert.match(container.textContent ?? '', /No recorded usage in this period/)
  assert.doesNotMatch(
    container.textContent ?? '',
    /All accounts: native Core usage/
  )
})
