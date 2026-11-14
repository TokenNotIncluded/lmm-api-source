/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, test } from 'node:test'

import { Window } from 'happy-dom'

const dom = new Window({ url: 'http://localhost/' })
dom.document.write('<!doctype html><html><body></body></html>')
// Exercise request cancellation with the user's reduced-motion preference;
// happy-dom's native animation cancellation does not match browser promises.
const reducedMotion = dom.matchMedia('(prefers-reduced-motion)')
Object.defineProperty(reducedMotion, 'matches', { value: true })
const matchMedia = dom.matchMedia.bind(dom)
Object.defineProperty(dom, 'matchMedia', {
  value: (query: string) =>
    query.includes('prefers-reduced-motion')
      ? reducedMotion
      : matchMedia(query),
})
for (const key of [
  'window',
  'document',
  'navigator',
  'localStorage',
  'HTMLElement',
  'customElements',
  'CSSStyleSheet',
  'HTMLButtonElement',
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
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: dom[key],
  })
}
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
after(() => dom.close())

const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { DetailsDialog } = await import('../dialogs/details-dialog')
const { usageLogSchema } = await import('../../data/schema')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')

const i18n = createInstance()
await i18n.use(initReactI18next).init({
  lng: 'en',
  fallbackLng: 'en',
  interpolation: { escapeValue: false },
  resources: { en: { translation: {} } },
})
const initialLog = usageLogSchema.parse({
  id: 1,
  user_id: 7,
  created_at: 1704067200,
  type: 2,
  content: '',
  request_id: 'request-current',
  model_name: 'model-a',
  state: 'settled',
  price_version: 'old-list-price',
})
const currentLog = {
  ...initialLog,
  price_version: 'current-price-v3',
  duration_ms: 1500.25,
  first_token_ms: 500.25,
  is_stream: true,
  completion_tokens: 100,
}

async function settle(predicate: () => boolean) {
  for (let index = 0; index < 60 && !predicate(); index += 1) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 10))
    })
  }
  assert.ok(predicate())
}

async function mount() {
  useAuthStore.getState().auth.setUser({ id: 7, username: 'reader', role: 1 })
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  await act(async () =>
    root.render(
      <I18nextProvider i18n={i18n}>
        <QueryClientProvider client={client}>
          <DetailsDialog
            open
            log={initialLog}
            isAdmin={false}
            onOpenChange={() => {}}
          />
        </QueryClientProvider>
      </I18nextProvider>
    )
  )
  return async () => {
    await act(async () => root.unmount())
    client.clear()
    container.remove()
    useAuthStore.getState().auth.reset('idle')
  }
}

test('opening a native log reads the current authorized receipt and its precise timings', async () => {
  const adapter = api.defaults.adapter
  const paths: string[] = []
  api.defaults.adapter = async (config) => {
    paths.push(config.url ?? '')
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: { success: true, data: currentLog },
    }
  }
  const cleanup = await mount()
  try {
    await settle(
      () => document.body.textContent?.includes('current-price-v3') === true
    )
    assert.deepEqual(paths, [
      '/core/v1/usage/self/request?request_id=request-current',
    ])
    assert.doesNotMatch(document.body.textContent ?? '', /old-list-price/)
    assert.match(document.body.textContent ?? '', /500.25 ms/)
    assert.match(
      document.body.textContent ?? '',
      /Generation speed \(estimated\).*100 t\/s/
    )
    assert.ok(document.querySelector('[aria-label="Copy Request ID"]'))
  } finally {
    await cleanup()
    api.defaults.adapter = adapter
  }
})

test('changing accounts aborts an open detail read and discards the old receipt', async () => {
  const adapter = api.defaults.adapter
  let release: (() => void) | undefined
  let oldSignal: AbortSignal | undefined
  const pending = new Promise<void>((resolve) => {
    release = resolve
  })
  api.defaults.adapter = async (config) => {
    if (config.authScope?.userId === 7) {
      oldSignal = config.signal as AbortSignal
      await pending
      return {
        config,
        headers: {},
        status: 200,
        statusText: 'OK',
        data: { success: true, data: currentLog },
      }
    }
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: { success: false },
    }
  }
  const cleanup = await mount()
  try {
    await settle(() => oldSignal !== undefined)
    await act(async () =>
      useAuthStore
        .getState()
        .auth.setUser({ id: 8, username: 'next-reader', role: 1 })
    )
    await settle(
      () =>
        document.body.textContent?.includes(
          'Unable to load request details'
        ) === true
    )
    assert.equal(oldSignal?.aborted, true)
    await act(async () => release?.())
    assert.doesNotMatch(
      document.body.textContent ?? '',
      /current-price-v3|old-list-price/
    )
  } finally {
    release?.()
    await cleanup()
    api.defaults.adapter = adapter
  }
})
