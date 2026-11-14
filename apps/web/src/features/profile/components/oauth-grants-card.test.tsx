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

test('managed cc-switch grants show device label and isolate cache by session', async () => {
  const { useAuthStore } = await import('@/stores/auth-store')
  const previousAuth = useAuthStore.getState().auth
  const baseFields = {
    scopes: ['catalog:read', 'balance:read', 'models:invoke'],
    budget_credits: 500000,
    created_at: 1_800_000_000,
    expires_at: 1_900_000_000,
    revoked_at: null,
    active: true,
  }
  let currentGrants: OAuthGrantSummary[] = [
    {
      ...baseFields,
      id: 'lmm_gr_device_a',
      client_id: 'lmm-cc-switch',
      purpose: 'normal_model',
      managed_application: 'cc-switch',
      installation_id: 'abcdef0123456789'.repeat(4),
      display_label: 'Workstation A',
    },
  ]
  let requests = 0
  const originalAdapter = api.defaults.adapter
  api.defaults.adapter = async (config) => {
    requests++
    return {
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: { success: true, data: currentGrants },
    }
  }
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 60_000 } },
  })
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  try {
    useAuthStore.getState().auth.setBundle({
      access_token: 'tok-a',
      token_type: 'Bearer',
      access_expires_at: 2_000_000_000,
      user: { id: 101, username: 'alice', role: 1 } as never,
      session: { sid: 'sess-a' } as never,
    })
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
      i < 50 && !host.textContent?.includes('Workstation A');
      i++
    ) {
      await act(async () => new Promise((resolve) => setTimeout(resolve, 10)))
    }
    assert.ok(
      host.textContent?.includes('CC Switch · Workstation A (abcdef01)')
    )
    assert.equal(requests, 1)

    currentGrants = [
      {
        ...baseFields,
        id: 'lmm_gr_device_b',
        client_id: 'lmm-cc-switch',
        purpose: 'normal_model',
        managed_application: 'cc-switch',
        installation_id: '9999888877776666'.repeat(4),
        display_label: 'Laptop B',
      },
    ]
    await act(async () => {
      useAuthStore.getState().auth.setBundle({
        access_token: 'tok-b',
        token_type: 'Bearer',
        access_expires_at: 2_000_000_000,
        user: { id: 202, username: 'bob', role: 1 } as never,
        session: { sid: 'sess-b' } as never,
      })
    })
    for (let i = 0; i < 50 && !host.textContent?.includes('Laptop B'); i++) {
      await act(async () => new Promise((resolve) => setTimeout(resolve, 10)))
    }
    assert.ok(host.textContent?.includes('CC Switch · Laptop B (99998888)'))
    assert.equal(requests, 2)
  } finally {
    await act(async () => root.unmount())
    useAuthStore.setState({ auth: previousAuth })
    queryClient.clear()
    host.remove()
    api.defaults.adapter = originalAdapter
  }
})

async function renderGrants(
  adapter: typeof api.defaults.adapter,
  run: (host: HTMLDivElement) => Promise<void>
) {
  const previous = api.defaults.adapter
  api.defaults.adapter = adapter
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
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
    await run(host)
  } finally {
    await act(async () => root.unmount())
    queryClient.clear()
    host.remove()
    api.defaults.adapter = previous
  }
}
async function until(check: () => boolean) {
  for (let i = 0; i < 60 && !check(); i++) {
    await act(async () => new Promise((resolve) => setTimeout(resolve, 10)))
  }
  assert.ok(check())
}

test('client revoke requires confirmation, sends only client id and refreshes all installations', async () => {
  let revoked = false
  let writes = 0
  const fields = {
    purpose: 'normal_model',
    scopes: ['models:invoke'],
    budget_credits: 0,
    created_at: 1_800_000_000,
    expires_at: 1_900_000_000,
    revoked_at: null,
    active: true,
  }
  await renderGrants(
    async (config) => {
      if (config.method === 'post') {
        writes++
        assert.equal(config.url, '/api/user/auth/oauth2/grants/revoke-client')
        assert.deepEqual(JSON.parse(config.data), {
          client_id: 'dynamic-client',
        })
        revoked = true
      }
      return {
        config,
        status: 200,
        statusText: 'OK',
        headers: {},
        data:
          config.method === 'post'
            ? { success: true, data: { revoked: 2 } }
            : {
                success: true,
                data: [
                  ...['first', 'second'].map((id) => ({
                    ...fields,
                    id,
                    client_id: 'dynamic-client',
                    active: !revoked,
                    revoked_at: revoked ? 1_800_000_001 : null,
                  })),
                  { ...fields, id: 'other', client_id: 'other-client' },
                ],
              },
      }
    },
    async (host) => {
      await until(() => host.querySelectorAll('section').length === 2)
      assert.match(host.textContent ?? '', /Budget: 0/)
      const section = host.querySelector('section[aria-label="dynamic-client"]')
      assert.ok(section)
      await act(async () =>
        (section.querySelector('button') as HTMLButtonElement).click()
      )
      assert.equal(writes, 0)
      await until(() => document.querySelector('[role="alertdialog"]') !== null)
      const dialog = document.querySelector('[role="alertdialog"]')
      assert.ok(dialog)
      assert.match(dialog.textContent ?? '', /dynamic-client/)
      const confirm = [...dialog.querySelectorAll('button')].find(
        (button) => button.textContent === 'Revoke access'
      )
      assert.ok(confirm)
      await act(async () => confirm.click())
      await until(() => section.textContent?.includes('Revoked') === true)
      assert.equal(writes, 1)
      assert.equal(
        host
          .querySelector('section[aria-label="other-client"] button')
          ?.hasAttribute('disabled'),
        false
      )
    }
  )
})

test('business failure is shown as an error, never as an empty grant list', async () => {
  await renderGrants(
    async (config) => ({
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: { success: false, message: 'Unavailable' },
    }),
    async (host) => {
      await until(
        () =>
          host.textContent?.includes('Unable to load authorized clients') ===
          true
      )
      assert.ok(!host.textContent?.includes('No authorized clients'))
      assert.ok(
        [...host.querySelectorAll('button')].some(
          (button) => button.textContent === 'Retry'
        )
      )
    }
  )
})
