/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, test } from 'node:test'

import { Window } from 'happy-dom'

const dom = new Window({
  url: 'https://console.example.test/rankings',
  settings: { device: { prefersReducedMotion: 'reduce' } },
})
dom.document.write('<!doctype html><html><head></head><body></body></html>')
const priorGlobals = new Map<string, PropertyDescriptor | undefined>()
for (const key of [
  'window',
  'customElements',
  'document',
  'navigator',
  'HTMLElement',
  'Element',
  'Node',
  'Event',
  'MouseEvent',
  'PointerEvent',
  'HTMLButtonElement',
  'HTMLCanvasElement',
  'ResizeObserver',
  'MutationObserver',
  'SVGElement',
  'getComputedStyle',
  'requestAnimationFrame',
  'cancelAnimationFrame',
  'matchMedia',
] as const) {
  priorGlobals.set(key, Object.getOwnPropertyDescriptor(globalThis, key))
  const value: unknown = dom[key]
  Object.defineProperty(globalThis, key, {
    configurable: true,
    writable: true,
    value:
      typeof value === 'function' && /^[a-z]/.test(key)
        ? value.bind(dom)
        : value,
  })
}
priorGlobals.set(
  'IS_REACT_ACT_ENVIRONMENT',
  Object.getOwnPropertyDescriptor(globalThis, 'IS_REACT_ACT_ENVIRONMENT')
)
Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
  configurable: true,
  writable: true,
  value: true,
})
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { createInstance } = await import('i18next')
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const { api } = await import('@/lib/api')
const { RankingsContent } = await import('./index')
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
after(() => {
  dom.close()
  for (const [key, descriptor] of priorGlobals) {
    if (descriptor) Object.defineProperty(globalThis, key, descriptor)
    else Reflect.deleteProperty(globalThis, key)
  }
})

async function until(check: () => boolean) {
  for (let i = 0; i < 80 && !check(); i++) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 8))
    })
  }
  assert.ok(check(), 'ranking panel did not reach the expected state')
}

test('model and user rankings recover independently; outage responses never appear as empty user rankings', async () => {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const root = createRoot(
    document.body.appendChild(document.createElement('div'))
  )
  const original = api.get
  let modelCalls = 0
  let userCalls = 0
  const modelSection = 'section[aria-label="Unable to load rankings"]'
  const userSection =
    'section[aria-label="User usage rankings are unavailable right now."]'
  api.get = (async (url, config) => {
    assert.equal(config?.skipErrorHandler, true)
    assert.equal(config?.skipBusinessError, true)
    assert.deepEqual(config?.params, { period: 'week' })
    if (url === '/api/rankings') {
      modelCalls++
      if (modelCalls === 3) {
        return {
          data: {
            success: false,
            error: { code: 'core_migration_incomplete' },
            data: { models: [] },
            message: 'private routing detail',
          },
        }
      }
      throw Object.assign(new Error('Request failed with status code 503'), {
        isAxiosError: true,
        response: {
          status: 503,
          data: [
            {
              success: false,
              code: 'RANKINGS_UNAVAILABLE',
              message: 'private diagnostic',
            },
            '<html>proxy error</html>',
          ][modelCalls - 1],
        },
      })
    }
    assert.equal(url, '/api/rankings/users')
    userCalls++
    if (userCalls === 1) {
      return {
        data: {
          success: false,
          code: 'EXTENSION_BUSY',
          data: { users: [] },
          message: 'private diagnostic',
        },
      }
    }
    return {
      data: {
        success: true,
        data: {
          period: 'week',
          updated_at: 1700000000,
          total_tokens: 17,
          total_requests: 1,
          participant_count: 1,
          anonymous_participant_count: 0,
          users: [
            {
              rank: 1,
              name: 'Visible participant',
              anonymous: false,
              total_tokens: 17,
              requests: 1,
              share: 1,
            },
          ],
        },
      },
    }
  }) as typeof api.get
  try {
    await act(async () =>
      root.render(
        <I18nextProvider i18n={i18n}>
          <QueryClientProvider client={client}>
            <RankingsContent
              compact={false}
              period='week'
              onPeriodChange={() => {}}
            />
          </QueryClientProvider>
        </I18nextProvider>
      )
    )
    await until(() => !!document.querySelector(modelSection))
    assert.match(
      document.querySelector(modelSection)?.textContent ?? '',
      /could not read usage data/
    )
    assert.equal(client.getQueryState(['rankings', 'week'])?.status, 'error')
    assert.equal(userCalls, 0)
    const opener = document.querySelector<HTMLButtonElement>(
      '[data-slot="collapsible-trigger"]'
    )
    assert.ok(opener)
    await act(async () => opener.click())
    await until(() => !!document.querySelector(userSection))
    assert.match(
      document.querySelector(userSection)?.textContent ?? '',
      /is busy/
    )
    assert.doesNotMatch(
      document.body.textContent ?? '',
      /No public usage data yet|Request failed with status code|private diagnostic/
    )
    assert.equal(
      client.getQueryState(['user-usage-rankings', 'week'])?.status,
      'error'
    )
    const retryUsers = document.querySelector<HTMLButtonElement>(
      `${userSection} button`
    )
    assert.ok(retryUsers)
    await act(async () => retryUsers.click())
    await until(
      () => !!document.body.textContent?.includes('Visible participant')
    )
    assert.equal(userCalls, 2)
    assert.equal(modelCalls, 1)
    const retryModels = document.querySelector<HTMLButtonElement>(
      `${modelSection} button`
    )
    assert.ok(retryModels)
    await act(async () => retryModels.click())
    await until(
      () =>
        !!document
          .querySelector(modelSection)
          ?.textContent?.includes('(HTTP 503)')
    )
    assert.equal(modelCalls, 2)
    assert.equal(userCalls, 2)
    assert.match(document.body.textContent ?? '', /Visible participant/)
    assert.doesNotMatch(
      document.body.textContent ?? '',
      /proxy error|could not read usage data/
    )
    await act(async () => retryModels.click())
    await until(
      () =>
        !!document
          .querySelector(modelSection)
          ?.textContent?.includes('Core does not serve this rankings endpoint')
    )
    assert.equal(modelCalls, 3)
    assert.equal(userCalls, 2)
    assert.doesNotMatch(
      document.body.textContent ?? '',
      /private routing detail/
    )
  } finally {
    await act(async () => root.unmount())
    client.clear()
    api.get = original
    document.body.replaceChildren()
  }
})
