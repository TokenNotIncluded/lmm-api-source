/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, test } from 'node:test'

import { Window } from 'happy-dom'

import type { OAuthGrantSummary } from '../api'

const dom = new Window({ url: 'https://grants.example.test/profile' })
const descriptors = new Map<string, PropertyDescriptor | undefined>()
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLButtonElement',
  'Element',
  'SVGElement',
  'Node',
  'Event',
  'MouseEvent',
  'CustomEvent',
  'MutationObserver',
  'ResizeObserver',
  'requestAnimationFrame',
  'cancelAnimationFrame',
  'getComputedStyle',
] as const) {
  descriptors.set(key, Object.getOwnPropertyDescriptor(globalThis, key))
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: dom[key],
  })
}
descriptors.set(
  'IS_REACT_ACT_ENVIRONMENT',
  Object.getOwnPropertyDescriptor(globalThis, 'IS_REACT_ACT_ENVIRONMENT')
)
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
const { OAuthGrantsCard } = await import('./oauth-grants-card')
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })

after(() => {
  dom.close()
  for (const [key, descriptor] of descriptors) {
    if (descriptor) Object.defineProperty(globalThis, key, descriptor)
    else Reflect.deleteProperty(globalThis, key)
  }
})

test('managed and legacy grants use their actual purpose for model versus read-only labels', async () => {
  const fields = {
    scopes: ['models:read'],
    budget_credits: null,
    created_at: 1_800_000_000,
    expires_at: 1_900_000_000,
    revoked_at: null,
    active: true,
  }
  const grants: OAuthGrantSummary[] = [
    {
      ...fields,
      id: 'managed-model',
      client_id: 'lmm-cc-switch',
      purpose: 'normal_model',
    },
    {
      ...fields,
      id: 'legacy-model',
      client_id: 'lmm',
      purpose: 'normal_model',
    },
    {
      ...fields,
      id: 'pi-read-only',
      client_id: 'lmm-pi',
      purpose: 'read_only',
    },
    {
      ...fields,
      id: 'ordinary-read-only',
      client_id: 'lmm',
      purpose: 'read_only',
    },
  ]
  let requests = 0
  const originalAdapter = api.defaults.adapter
  api.defaults.adapter = async (config) => {
    requests++
    assert.equal(config.method, 'get')
    assert.equal(config.url, '/api/user/auth/oauth2/grants')
    return {
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: { success: true, data: grants },
    }
  }
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  try {
    await act(async () =>
      root.render(
        <QueryClientProvider client={queryClient}>
          <I18nextProvider i18n={i18n}>
            <OAuthGrantsCard />
          </I18nextProvider>
        </QueryClientProvider>
      )
    )
    for (
      let i = 0;
      i < 50 && host.querySelectorAll('p.font-medium').length !== 4;
      i++
    ) {
      await act(async () => new Promise((resolve) => setTimeout(resolve, 10)))
    }
    const labels = [...host.querySelectorAll('p.font-medium')]
    assert.deepEqual(
      labels.map((p) => p.textContent),
      ['Model client', 'Model client', 'Read-only client', 'Read-only client']
    )
    assert.deepEqual(
      labels.map((p) => p.nextElementSibling?.textContent),
      [
        'Model access',
        'Model access',
        'Read-only account access',
        'Read-only account access',
      ]
    )
    assert.equal(requests, 1)
    assert.equal(
      [...host.querySelectorAll('button')].filter(
        (b) => b.textContent === 'Revoke'
      ).length,
      4
    )
    assert.equal(host.querySelectorAll('[role="dialog"]').length, 0)
  } finally {
    await act(async () => root.unmount())
    queryClient.clear()
    host.remove()
    api.defaults.adapter = originalAdapter
  }
})
