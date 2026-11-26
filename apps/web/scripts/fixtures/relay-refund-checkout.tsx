/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { createInstance } from 'i18next'
import { createRoot } from 'react-dom/client'
import { I18nextProvider } from 'react-i18next'

import { RelayCheckout } from '../../src/features/tool-market/relay-checkout'
import { api } from '../../src/lib/api'
import { useAuthStore } from '../../src/stores/auth-store'

const orderID = 'a'.repeat(64)
const path = '/api/ssmcpd/orders/' + orderID
const payload = {
  order_id: orderID,
  user_id: 42,
  gib: 2,
  amount_usd_cents: 200,
  period: '2026-11',
  state: 'preview',
  refund_ref: '',
}
let reads = 0,
  confirmations = 0
api.defaults.adapter = async (config) => {
  if (config.url === path && config.method === 'get') {
    reads++
    return {
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: { success: true, data: { ...payload } },
    }
  }
  if (config.url === path + '/confirm' && config.method === 'post') {
    confirmations++
    payload.state = 'refund_required'
    throw new Error('Synthetic lost confirmation response')
  }
  throw new Error('Unexpected request: ' + config.method + ' ' + config.url)
}

function signIn(id: number) {
  const now = Math.floor(Date.now() / 1000)
  useAuthStore.getState().auth.setBundle({
    access_token: 'synthetic-' + id,
    token_type: 'Bearer',
    access_expires_at: now + 3600,
    user: { id, role: 1, username: 'fixture-' + id, quota: 0 },
    session: {
      sid: 'fixture-' + id,
      current: true,
      login_method: 'fixture',
      ip: '127.0.0.1',
      user_agent: 'fixture',
      created_at: now,
      last_active_at: now,
      expires_at: now + 3600,
    },
  })
}
const visible = (label: string) => document.body.textContent?.includes(label)
const button = (name: string) =>
  [...document.querySelectorAll('button')].find(
    (b) => b.textContent?.trim() === name
  )
const support = () =>
  document.querySelector<HTMLAnchorElement>('a[href*="/support?"]')
const requireThat = (condition: unknown, message: string) => {
  if (!condition) {
    throw new Error(message)
  }
}
async function until(condition: () => unknown, label: string) {
  const deadline = Date.now() + 15000
  while (!condition()) {
    if (Date.now() > deadline) {
      throw new Error(
        'Timed out: ' + label + ' (' + document.body.textContent + ')'
      )
    }
    await new Promise((r) => setTimeout(r, 30))
  }
}
async function testRefundCheckout() {
  const i18n = createInstance()
  await i18n.init({
    lng: 'en',
    resources: { en: { translation: {} } },
    fallbackLng: 'en',
  })
  signIn(42)
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  createRoot(document.getElementById('root')!).render(
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={client}>
        <RelayCheckout orderID={orderID} />
      </QueryClientProvider>
    </I18nextProvider>
  )
  await until(() => button('Pay with account balance'), 'initial order')
  button('Pay with account balance')!.click()
  await until(() => visible('Refund required') && support(), 'refund required')
  requireThat(
    !button('Retry') &&
      !button('Pay with account balance') &&
      !visible('Check its status before retrying'),
    'refund prompted another confirmation'
  )
  const url = new URL(support()!.href)
  requireThat(
    url.origin === location.origin &&
      url.pathname === '/support' &&
      url.searchParams.get('category') === 'refund' &&
      url.searchParams.get('referenceId') === orderID,
    'support ticket lacks the local order reference'
  )
  requireThat(!visible('Refund reference'), 'invented refund receipt')
  payload.state = 'refund_pending'
  button('Refresh')!.click()
  await until(() => visible('Refund pending'), 'refund in progress')
  requireThat(
    !button('Retry') && !button('Pay with account balance'),
    'pending refund prompted payment'
  )
  payload.state = 'refunded'
  payload.refund_ref = 'fixture-refund-42'
  button('Refresh')!.click()
  await until(
    () => visible('Refunded') && visible('fixture-refund-42'),
    'returned refund reference'
  )
  const previousReads = reads
  signIn(88)
  await until(
    () => reads > previousReads && document.querySelector('[role=alert]'),
    'wrong-owner response rejected'
  )
  requireThat(
    !visible('fixture-refund-42') && !support() && !visible(orderID),
    'order data remained visible to another account'
  )
  requireThat(confirmations === 1, 'refresh sent a payment request')
}
testRefundCheckout()
  .then(() => fetch('/result', { method: 'POST', body: 'PASS' }))
  .catch((error) => fetch('/result', { method: 'POST', body: String(error) }))
