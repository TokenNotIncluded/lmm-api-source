/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, test } from 'node:test'

import { Window } from 'happy-dom'

const dom = new Window({ url: 'https://synthetic.example/profile' })
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLInputElement',
  'SVGElement',
  'Node',
  'Element',
  'Event',
  'CustomEvent',
  'MutationObserver',
  'ResizeObserver',
  'requestAnimationFrame',
  'cancelAnimationFrame',
  'localStorage',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: dom[key],
  })
}
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { SmsHistoryCard } = await import('./sms-history-card')
const { readSmsHistory } = await import('./sms-history')
const originalAdapter = api.defaults.adapter
const originalAuth = useAuthStore.getState().auth
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
after(() => {
  api.defaults.adapter = originalAdapter
  useAuthStore.setState({ auth: originalAuth })
  dom.close()
})

function login(id: number) {
  useAuthStore.setState({
    auth: {
      ...originalAuth,
      user: { id, username: `synthetic-${id}`, role: 1 },
      accessToken: `synthetic-bearer-${id}`,
      accessExpiresAt: Math.floor(Date.now() / 1000) + 3600,
      bootstrapState: 'complete',
      session: {
        sid: `synthetic-session-${id}`,
        current: true,
        login_method: 'password',
        ip: null,
        user_agent: null,
        created_at: 1,
        last_active_at: 1,
        expires_at: Math.floor(Date.now() / 1000) + 3600,
      },
    },
  })
}
function fixture(page: number, owner: number) {
  return {
    success: true,
    data: {
      contract: 'core.hero-sms-history.v1',
      observed_at: 1793674367,
      page,
      page_size: 20,
      total: 21,
      quota_unit: 'legacy_raw_no_conversion',
      payload_available: false,
      supplier_operations_available: false,
      items: Array.from({ length: page === 1 ? 20 : 1 }, (_, n) => ({
        id: `source-${owner}-${page}-${n}`,
        service: 'synthetic-service',
        country_id: 1,
        operator: 'synthetic',
        status: 'completed',
        quoted_price_usd: '0.123456789123456789',
        raw_reserved_units: '9007199254741009',
        raw_refunded_units: '0',
        raw_net_debit_units: '9007199254741009',
        created_at: 1793600000,
        completed_at: 1793600001,
        ledger: [
          {
            entry_type: 'reserve',
            raw_amount_units: '-9007199254741009',
            created_at: 1793600000,
          },
        ],
      })),
    },
  }
}
async function until(check: () => boolean) {
  const end = Date.now() + 3000
  while (!check()) {
    if (Date.now() > end) throw new Error('UI condition timed out')
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 10))
    })
  }
}
test('SMS history paginates exactly, rejects unknown units, and discards old-account responses', async () => {
  login(71)
  let mode: 'valid' | 'invalid' | 'deferred' | 'empty' = 'valid'
  let release = () => {}
  let requestedPage = 0
  api.defaults.adapter = async (config) => {
    assert.equal(config.url, '/core/v1/hero-sms/history')
    assert.equal(config.method, 'get')
    requestedPage = Number(config.params.p)
    const body = fixture(
      requestedPage,
      useAuthStore.getState().auth.user?.id ?? 0
    )
    if (mode === 'invalid') body.data.quota_unit = 'core_credit'
    if (mode === 'empty') {
      body.data.total = 0
      body.data.items = []
    }
    if (mode === 'deferred') {
      await new Promise<void>((resolve) => {
        release = resolve
      })
    }
    return { data: body, status: 200, statusText: 'OK', headers: {}, config }
  }
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  const click = async (label: string) => {
    const button = [...container.querySelectorAll('button')].find(
      (b) => b.textContent === label
    )
    assert.ok(button)
    await act(async () => {
      button.click()
    })
  }
  try {
    await act(async () => {
      root.render(
        <QueryClientProvider client={client}>
          <I18nextProvider i18n={i18n}>
            <SmsHistoryCard />
          </I18nextProvider>
        </QueryClientProvider>
      )
    })
    await until(() => container.textContent?.includes('source-71-1-0') === true)
    assert.ok(container.textContent?.includes('9,007,199,254,741,009'))
    assert.ok(container.textContent?.includes('0.123456789123456789'))
    assert.equal(
      container.querySelectorAll('button').length,
      2,
      'only pagination actions'
    )
    await click('Next page')
    await until(() => container.textContent?.includes('source-71-2-0') === true)
    assert.equal(requestedPage, 2)
    assert.ok(!container.textContent?.includes('source-71-1-0'))
    mode = 'invalid'
    await click('Previous page')
    await until(() => container.querySelector('[role="alert"]') !== null)
    assert.ok(!container.textContent?.includes('source-71-'))
    assert.ok(
      !container.textContent?.includes('No visible archived SMS orders.')
    )
    mode = 'valid'
    await click('Retry')
    await until(() => container.textContent?.includes('source-71-1-0') === true)
    mode = 'deferred'
    await click('Next page')
    await until(() => requestedPage === 2)
    mode = 'empty'
    await act(async () => {
      login(72)
    })
    await until(
      () =>
        container.textContent?.includes('No visible archived SMS orders.') ===
        true
    )
    await act(async () => {
      release()
      await new Promise((resolve) => setTimeout(resolve, 10))
    })
    assert.ok(!container.textContent?.includes('source-71-'))
    assert.equal(requestedPage, 1, 'account change resets pagination')
    await assert.rejects(
      readSmsHistory({ userId: 71, sessionId: 'synthetic-session-71' }, 1)
    )
  } finally {
    await act(async () => {
      root.unmount()
    })
    client.clear()
    container.remove()
  }
})
