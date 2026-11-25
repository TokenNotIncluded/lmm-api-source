/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import type { AxiosAdapter } from 'axios'
import { Window } from 'happy-dom'

import type { CoreConfigurationDomain } from './core-configuration-section'

const dom = new Window({ url: 'https://settings.example' })
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'Element',
  'Node',
  'self',
  'location',
  'history',
  'sessionStorage',
  'MutationObserver',
  'getComputedStyle',
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

Object.defineProperty(globalThis, 'CustomEvent', {
  configurable: true,
  value: window.CustomEvent,
})
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { ROLE } = await import('@/lib/roles')
const { useAuthStore } = await import('@/stores/auth-store')
const { CoreConfigurationSection } =
  await import('./core-configuration-section')

const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })

function signIn(role: number) {
  useAuthStore.getState().auth.setBundle({
    user: { id: 41, username: 'fixture-owner', role },
    access_token: 'synthetic',
    token_type: 'Bearer',
    access_expires_at: 9999999999,
    session: {
      sid: 'fixture-session',
      current: true,
      login_method: 'password',
      ip: null,
      user_agent: null,
      created_at: 1,
      last_active_at: 1,
      expires_at: 9999999999,
    },
  })
}
async function renderSection(
  domain: CoreConfigurationDomain,
  adapter: AxiosAdapter,
  inspect: (host: HTMLDivElement) => Promise<void>
) {
  const old = api.defaults.adapter
  api.defaults.adapter = adapter
  const host = document.body.appendChild(document.createElement('div'))
  const root = createRoot(host)
  const client = new QueryClient()
  try {
    await act(async () =>
      root.render(
        <QueryClientProvider client={client}>
          <I18nextProvider i18n={i18n}>
            <CoreConfigurationSection domain={domain} />
          </I18nextProvider>
        </QueryClientProvider>
      )
    )
    await inspect(host)
  } finally {
    await act(async () => root.unmount())
    client.clear()
    host.remove()
    api.defaults.adapter = old
  }
}
async function waitFor(host: HTMLElement, text: string) {
  for (let i = 0; i < 100 && !host.textContent?.includes(text); i++) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 10))
    })
  }
  assert.ok(host.textContent?.includes(text), host.textContent ?? '')
}

test('console reads Core alone, preserving an empty notice and the canonical announcement alias', async () => {
  signIn(ROLE.SUPER_ADMIN)
  const paths: string[] = []
  await renderSection(
    'console',
    async (cfg) => {
      paths.push(`${cfg.method} ${cfg.url}`)
      const data = {
        success: true,
        capabilities: { read_only: true, partial: true, writable_keys: [] },
        data: [
          {
            key: 'Notice',
            value: '',
            source: 'legacy_database',
            revision: '172',
          },
          { key: 'Announcements', value: '[]' },
          {
            key: 'console_setting.announcements',
            value: JSON.stringify([
              {
                id: 1,
                content: 'Current publication',
                publishDate: '2026-01-01T00:00:00Z',
              },
            ]),
          },
        ],
      }
      return { config: cfg, status: 200, statusText: 'OK', headers: {}, data }
    },
    async (host) => {
      await waitFor(host, 'Current publication')
      assert.match(host.textContent ?? '', /Server AddressUnavailable/)
      assert.match(host.textContent ?? '', /System NoticeNot configured/)
      assert.deepEqual(paths, ['get /api/option/'])
      assert.equal(host.querySelector('input, textarea'), null)
      assert.match(host.textContent ?? '', /legacy_database/)
      assert.match(host.textContent ?? '', /172/)
      const buttons = [...host.querySelectorAll('button')]
      assert.deepEqual(
        buttons.map((button) => button.textContent),
        ['Ask AI to edit']
      )
      const { consumeQueuedAssistantRequest } =
        await import('@/features/assistant/assistant-events')
      await act(async () => buttons[0].click())
      const draft = consumeQueuedAssistantRequest()
      assert.equal(draft?.autoSend, false)
      assert.match(draft?.message ?? '', /Git/)
      assert.match(draft?.message ?? '', /\/api\/option\//)
      assert.ok(!draft?.message?.includes('Current publication'))
      assert.deepEqual(paths, ['get /api/option/'])
    }
  )
})

test('routing reads independent revisions, reports a denied group request, and retries just that request', async () => {
  signIn(ROLE.SUPER_ADMIN)
  const paths: string[] = []
  let first = true
  await renderSection(
    'routing',
    async (cfg) => {
      paths.push(`${cfg.method} ${cfg.url}`)
      if (cfg.url?.endsWith('/auto-groups') && first) {
        first = false
        throw Object.assign(new Error('Forbidden'), {
          response: { status: 403 },
        })
      }
      const data = cfg.url?.endsWith('/auto-groups')
        ? { version: 1, revision: 9, groups: ['paid', 'default'] }
        : {
            version: 1,
            revision: 7,
            policy: {
              enabled: true,
              window_seconds: 60,
              user_limit: null,
              group_limits: { paid: 8 },
            },
          }
      return { config: cfg, status: 200, statusText: 'OK', headers: {}, data }
    },
    async (host) => {
      await waitFor(host, 'Access denied')
      await waitFor(host, 'No limit')
      assert.ok(host.textContent?.includes('paid8'))
      const retry = [...host.querySelectorAll('button')].find(
        (b) => b.textContent === 'Retry'
      )
      assert.ok(retry)
      await act(async () => retry.click())
      await waitFor(host, 'Revision 9')
      assert.deepEqual(
        [...host.querySelectorAll('ol li')].map((el) => el.textContent),
        ['paid', 'default']
      )
      assert.equal(
        paths.filter((p) => p === 'get /core/v1/admin/request-limits').length,
        1
      )
      assert.equal(
        paths.filter((p) => p === 'get /core/v1/admin/routing/auto-groups')
          .length,
        2
      )
      assert.equal(paths.length, 3)
    }
  )
})

test('a non-root user gets a permission outcome without a private read', async () => {
  signIn(ROLE.ADMIN)
  let requests = 0
  await renderSection(
    'billing',
    async () => {
      requests++
      throw new Error('unexpected read')
    },
    async (host) => {
      await waitFor(host, 'Super administrator access required.')
      assert.equal(requests, 0)
    }
  )
})
