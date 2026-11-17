/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import {
  act,
  api,
  flushEffects,
} from '@/features/assistant/assistant-key-tool-test-support'

// Use Motion's JS fallback: Happy DOM rejects canceled WAAPI animations during
// unmount. This test checks the mounted support page, not animation timing.
Object.defineProperty(HTMLElement.prototype, 'animate', {
  configurable: true,
  value: undefined,
})

const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const { createInstance } = await import('i18next')
const { I18nextProvider } = await import('react-i18next')
const { useAuthStore } = await import('@/stores/auth-store')
const { SupportTicket } = await import('./index')

test('support retains online tickets and request copying without email actions', async () => {
  const previous = useAuthStore.getState().auth
  const i18n = createInstance()
  await i18n.init({ lng: 'en', resources: { en: { translation: {} } } })
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const container = document.body.appendChild(document.createElement('div'))
  const root = createRoot(container)
  const originalGet = api.get
  api.get = (async (url: string) => {
    assert.ok(
      url === '/api/assistant/support/self' ||
        url === '/api/assistant/support/eligibility',
      `Unexpected support read: ${url}`
    )
    return {
      data: {
        success: true,
        data: url.endsWith('/self') ? { request: null } : { eligible: false },
      },
    }
  }) as typeof api.get
  try {
    useAuthStore.setState({
      auth: {
        ...previous,
        user: JSON.parse(JSON.stringify({
          id: 71,
          username: 'support-test',
          role: 1,
          developer_access_granted: false,
        })),
      },
    })
    await act(async () => {
      root.render(
        <QueryClientProvider client={client}>
          <I18nextProvider i18n={i18n}>
            <SupportTicket />
          </I18nextProvider>
        </QueryClientProvider>
      )
      await flushEffects()
    })
    const html = container.innerHTML
    assert.match(html, /data-testid="support-conversation"/)
    assert.match(html, /Copy request/)
    assert.doesNotMatch(
      html,
      /mailto:|Open email to submit|Email support directly|structured email/
    )
  } finally {
    await act(async () => root.unmount())
    useAuthStore.setState({ auth: previous })
    api.get = originalGet
    client.clear()
    container.remove()
  }
})
