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
const { CheckinHistoryCard } = await import('./checkin-history-card')
const { readCheckinHistory } = await import('../checkin-history')
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
function fixture(month: string, empty = false) {
  return {
    success: true,
    data: {
      contract: 'core.checkin-history.v1',
      month,
      observed_at: 1793674367,
      total_checkins: empty ? 0 : 2,
      total_raw_quota_awarded: empty ? '0' : '9007199254741009',
      award_unit: 'legacy_raw_no_conversion',
      native_claim_available: false,
      calendar_timezone: null,
      records: empty
        ? []
        : [
            {
              checkin_date: `${month}-02`,
              raw_quota_awarded: 42,
              source_created_at: 1793500000,
            },
          ],
    },
  }
}
async function until(check: () => boolean) {
  const deadline = Date.now() + 3000
  while (!check()) {
    assert.ok(Date.now() < deadline, 'UI did not reach expected state')
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 10))
    })
  }
}

test('history preserves exact units, filters months, rejects invalid data and clears on account changes', async () => {
  login(7)
  let mode: 'history' | 'invalid' | 'deferred' | 'empty' = 'history'
  let release = () => {}
  let requestedMonth = ''
  api.defaults.adapter = async (config) => {
    assert.equal(config.url, '/core/v1/checkins/history')
    requestedMonth = String(config.params.month)
    const data = fixture(requestedMonth, mode === 'empty')
    if (mode === 'invalid') {
      data.data.award_unit = 'core_credit'
    }
    if (mode === 'deferred') {
      await new Promise<void>((resolve) => {
        release = resolve
      })
    }
    return { data, status: 200, statusText: 'OK', headers: {}, config }
  }
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  try {
    await act(async () => {
      root.render(
        <QueryClientProvider client={client}>
          <I18nextProvider i18n={i18n}>
            <CheckinHistoryCard />
          </I18nextProvider>
        </QueryClientProvider>
      )
    })
    await until(
      () => container.textContent?.includes('9,007,199,254,741,009') === true
    )
    assert.ok(container.textContent?.includes('not your current balance'))
    assert.equal(
      container.querySelectorAll('button').length,
      0,
      'history offers no reward action'
    )
    const input = container.querySelector('input')
    assert.ok(input)
    const setValue = Object.getOwnPropertyDescriptor(
      dom.HTMLInputElement.prototype,
      'value'
    )?.set
    assert.ok(setValue)
    const changeMonth = async (month: string) => {
      await act(async () => {
        setValue.call(input, month)
        input.dispatchEvent(new Event('change', { bubbles: true }))
        input.dispatchEvent(new Event('input', { bubbles: true }))
      })
    }
    await changeMonth('2026-10')
    await until(
      () => container.querySelector('time')?.textContent === '2026-10-02'
    )
    assert.equal(requestedMonth, '2026-10')
    mode = 'invalid'
    await changeMonth('2026-09')
    await until(() => container.querySelector('[role="alert"]') !== null)
    assert.ok(!container.textContent?.includes('9,007,199,254,741,009'))
    assert.ok(!container.textContent?.includes('No check-ins'))
    mode = 'history'
    await act(async () => {
      container.querySelector('button')?.click()
    })
    await until(
      () => container.querySelector('time')?.textContent === '2026-09-02'
    )
    mode = 'deferred'
    await changeMonth('2026-08')
    await until(() => requestedMonth === '2026-08')
    mode = 'empty'
    await act(async () => {
      login(8)
    })
    await until(
      () =>
        container.textContent?.includes('No check-ins in this month.') === true
    )
    await act(async () => {
      release()
      await new Promise((resolve) => setTimeout(resolve, 20))
    })
    assert.ok(!container.textContent?.includes('9,007,199,254,741,009'))
    assert.ok(!container.querySelector('time'))
    await assert.rejects(
      readCheckinHistory(
        { userId: 7, sessionId: 'synthetic-session-7' },
        '2026-11'
      ),
      /session changed/
    )
    await act(async () => {
      useAuthStore.setState({ auth: originalAuth })
    })
    assert.equal(container.textContent, '')
  } finally {
    await act(async () => root.unmount())
    client.clear()
    container.remove()
  }
})
