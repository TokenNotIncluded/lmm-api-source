/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { createInstance } from 'i18next'
import { renderToStaticMarkup } from 'react-dom/server'
import { I18nextProvider } from 'react-i18next'

import { useAuthStore } from '@/stores/auth-store'

import { SupportTicket } from './index'

test('support retains online tickets and request copying without email actions', async () => {
  const previous = useAuthStore.getState().auth
  const i18n = createInstance()
  await i18n.init({ lng: 'en', resources: { en: { translation: {} } } })
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  try {
    useAuthStore.setState({
      auth: {
        ...previous,
        user: {
          id: 71,
          username: 'support-test',
          role: 1,
          developer_access_granted: false,
        },
      },
    })
    const html = renderToStaticMarkup(
      <QueryClientProvider client={client}>
        <I18nextProvider i18n={i18n}>
          <SupportTicket />
        </I18nextProvider>
      </QueryClientProvider>
    )
    assert.match(html, /data-testid="support-conversation"/)
    assert.match(html, /Copy request/)
    assert.doesNotMatch(
      html,
      /mailto:|Open email to submit|Email support directly|structured email/
    )
  } finally {
    useAuthStore.setState({ auth: previous })
    client.clear()
  }
})
