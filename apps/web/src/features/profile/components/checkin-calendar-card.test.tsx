/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.

For commercial licensing, please contact support@quantumnous.com
*/
import assert from 'node:assert/strict'
import { after, afterEach, describe, test } from 'node:test'

import { Window } from 'happy-dom'

const domWindow = new Window()
domWindow.document.write(
  '<!doctype html><html><head></head><body></body></html>'
)
Object.defineProperty(domWindow.document, 'compatMode', {
  configurable: true,
  value: 'CSS1Compat',
})
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'SVGElement',
  'Node',
  'Element',
  'Event',
  'CustomEvent',
  'MutationObserver',
  'requestAnimationFrame',
  'cancelAnimationFrame',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: domWindow[key],
  })
}

const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { toast } = await import('sonner')
const { useAuthStore } = await import('@/stores/auth-store')
const { DEFAULT_CURRENCY_CONFIG, useSystemConfigStore } =
  await import('@/stores/system-config-store')
const { useWalletCurrencyPreferenceStore } =
  await import('@/stores/wallet-currency-preference-store')
const { CheckinCalendarCard } = await import('./checkin-calendar-card')

const originalGet = api.get
const reactTestGlobals = globalThis as typeof globalThis & {
  IS_REACT_ACT_ENVIRONMENT?: boolean
}
reactTestGlobals.IS_REACT_ACT_ENVIRONMENT = true

const i18n = createInstance()
await i18n.use(initReactI18next).init({
  lng: 'zh-CN',
  resources: {
    'zh-CN': {
      translation: {
        'Checked in': '已签到',
        'Next month': '下个月',
        'Previous month': '上个月',
      },
    },
  },
})

async function waitForCondition(
  condition: () => boolean,
  failureMessage: string
): Promise<void> {
  const deadline = Date.now() + 1500
  while (!condition()) {
    if (Date.now() >= deadline) {
      throw new Error(`${failureMessage}: ${document.body.textContent}`)
    }
    await new Promise((resolve) => setTimeout(resolve, 5))
  }
}

afterEach(() => {
  api.get = originalGet
  document.body.replaceChildren()
})

after(() => domWindow.close())

describe('check-in calendar accessibility', () => {
  test('renders localized, non-actionable dates and bounded month navigation', async () => {
    const now = new Date()
    const recordDate = `${now.getFullYear()}-${String(
      now.getMonth() + 1
    ).padStart(2, '0')}-01`
    api.get = (async () => ({
      data: {
        success: true,
        data: {
          enabled: true,
          stats: {
            checked_in_today: false,
            total_checkins: 1,
            total_quota: 100,
            checkin_count: 1,
            records: [{ checkin_date: recordDate, quota_awarded: 100 }],
          },
        },
      },
    })) as typeof api.get

    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    })
    const container = document.createElement('div')
    document.body.append(container)
    const root = createRoot(container)

    await act(async () => {
      root.render(
        <QueryClientProvider client={queryClient}>
          <I18nextProvider i18n={i18n}>
            <CheckinCalendarCard
              checkinEnabled
              turnstileEnabled={false}
              turnstileSiteKey=''
            />
          </I18nextProvider>
        </QueryClientProvider>
      )
    })
    await act(async () => {
      await waitForCondition(
        () => container.querySelector('[role="grid"]') !== null,
        'calendar grid did not render'
      )
    })

    const grid = container.querySelector('[role="grid"]')
    assert.ok(grid)
    assert.equal(grid.querySelectorAll('[role="gridcell"] button').length, 0)
    assert.deepEqual(
      [...grid.querySelectorAll('[role="columnheader"]')]
        .slice(0, 2)
        .map((header) => header.textContent),
      ['周日', '周一']
    )

    const checkedCell = grid.querySelector('[aria-label*="已签到"]')
    assert.ok(checkedCell)
    assert.equal(checkedCell.tagName, 'DIV')
    assert.equal(checkedCell.getAttribute('tabindex'), '0')

    const previousButton = container.querySelector<HTMLButtonElement>(
      'button[aria-label="上个月"]'
    )
    const nextButton = container.querySelector<HTMLButtonElement>(
      'button[aria-label="下个月"]'
    )
    assert.ok(previousButton)
    assert.ok(nextButton)
    assert.match(previousButton.className, /\bsize-11\b/)
    assert.equal(nextButton.disabled, true)

    await act(async () => {
      previousButton.click()
      await waitForCondition(
        () =>
          container.querySelector<HTMLButtonElement>(
            'button[aria-label="下个月"]'
          )?.disabled === false,
        'next-month navigation did not become available'
      )
    })

    await act(async () => root.unmount())
    queryClient.clear()
  })
})

test('check-in success uses the newly selected currency without remounting', async () => {
  const previousAuth = useAuthStore.getState().auth
  const previousCurrency = useSystemConfigStore.getState().config.currency
  const previousPreference =
    useWalletCurrencyPreferenceStore.getState().preference
  const previousPost = api.post
  const previousSuccess = toast.success
  let successMessage = ''
  toast.success = ((message: unknown) => {
    successMessage = String(message)
    return 1
  }) as typeof toast.success
  useAuthStore.getState().auth.setUser(null)
  useWalletCurrencyPreferenceStore.getState().setPreference('USD')
  useSystemConfigStore.getState().setConfig({
    currency: {
      ...DEFAULT_CURRENCY_CONFIG,
      currencyUnit: 'credit',
      creditsPerUsd: 500_000,
      creditsPerUsdExact: '500000',
      cnyPerUsd: 7,
      cnyPerUsdExact: '7',
    },
  })
  api.get = (async () => ({
    data: {
      success: true,
      data: {
        enabled: true,
        stats: {
          checked_in_today: false,
          total_checkins: 0,
          total_quota: 0,
          checkin_count: 0,
          records: [],
        },
      },
    },
  })) as typeof api.get
  api.post = (async () => ({
    data: { success: true, data: { quota_awarded: 3_500_000 } },
  })) as typeof api.post
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  try {
    await act(async () =>
      root.render(
        <QueryClientProvider client={client}>
          <I18nextProvider i18n={i18n}>
            <CheckinCalendarCard
              checkinEnabled
              turnstileEnabled={false}
              turnstileSiteKey=''
            />
          </I18nextProvider>
        </QueryClientProvider>
      )
    )
    await act(async () =>
      waitForCondition(
        () => host.querySelector('[role="grid"]') !== null,
        'calendar not loaded'
      )
    )
    await act(async () =>
      useWalletCurrencyPreferenceStore.getState().setPreference('CNY')
    )
    const button = [...host.querySelectorAll<HTMLButtonElement>('button')].find(
      (element) => element.textContent?.trim() === 'Check in now'
    )
    assert.ok(button, 'check-in button not found')
    await act(async () => {
      button.click()
      await waitForCondition(
        () => Boolean(successMessage),
        'check-in success not shown'
      )
    })
    assert.match(successMessage, /49 CNY/)
    assert.doesNotMatch(successMessage, /1 USD|Platform|\$/)
  } finally {
    await act(async () => root.unmount())
    client.clear()
    host.remove()
    api.post = previousPost
    toast.success = previousSuccess
    useAuthStore.setState({ auth: previousAuth })
    useSystemConfigStore.getState().setConfig({ currency: previousCurrency })
    useWalletCurrencyPreferenceStore
      .getState()
      .setPreference(previousPreference)
  }
})

const nativeZeroRecord = {
  checkin_date: '2030-01-01',
  quota_awarded: '0',
  created_at: 1893427200,
  journal_id: null,
  origin: 'history',
  policy_revision: null,
  trust_level: null,
}
function nativeStatus(checked: boolean, month = '2030-01') {
  return {
    enabled: true,
    today: '2030-01-01',
    month,
    calendar_offset_seconds: 28800,
    policy_revision: 1,
    checked_in_today: checked,
    reward_range: {
      trust_level: 0,
      reward_multiplier: 0,
      base_min_quota: '1',
      base_max_quota: '500000',
      min_quota: '0',
      max_quota: '0',
    },
    today_record: checked ? nativeZeroRecord : null,
    records: checked && month === '2030-01' ? [nativeZeroRecord] : [],
    total_checkins: checked ? 1 : 0,
    total_quota_awarded: '0',
  }
}
async function mountNativeCalendar() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  await act(async () =>
    root.render(
      <QueryClientProvider client={client}>
        <I18nextProvider i18n={i18n}>
          <CheckinCalendarCard
            checkinEnabled
            turnstileEnabled={false}
            turnstileSiteKey=''
          />
        </I18nextProvider>
      </QueryClientProvider>
    )
  )
  await act(async () =>
    waitForCondition(
      () => host.querySelector('button[aria-expanded]') !== null,
      'Native calendar did not load'
    )
  )
  return {
    host,
    close: async () => {
      await act(async () => root.unmount())
      client.clear()
      host.remove()
    },
  }
}

test('Native history zero blocks today and uses server month instead of browser month', async () => {
  const urls: string[] = []
  api.get = (async (url) => {
    urls.push(String(url))
    const month =
      new URL(String(url), 'http://local').searchParams.get('month') ??
      '2030-01'
    return { data: { success: true, data: nativeStatus(true, month) } }
  }) as typeof api.get
  const calendar = await mountNativeCalendar()
  try {
    assert.equal(urls[0], '/api/user/checkin')
    const button = [
      ...calendar.host.querySelectorAll<HTMLButtonElement>('button'),
    ].find((element) => element.textContent?.trim() === '已签到')
    assert.ok(button?.disabled)
    const toggle = calendar.host.querySelector<HTMLButtonElement>(
      'button[aria-expanded]'
    )
    assert.ok(toggle)
    await act(async () => toggle.click())
    const current = calendar.host.querySelector('[aria-current="date"]')
    assert.equal(current?.textContent?.trim(), '1')
    assert.match(current?.getAttribute('aria-label') ?? '', /2030.*已签到/)
    assert.match(calendar.host.textContent ?? '', /2030/)
    const previous = calendar.host.querySelector<HTMLButtonElement>(
      'button[aria-label="上个月"]'
    )
    assert.ok(previous)
    await act(async () => {
      previous.click()
      await waitForCondition(
        () => urls.some((url) => url.endsWith('month=2029-12')),
        'server month navigation failed'
      )
    })
  } finally {
    await calendar.close()
  }
})

test('Native lost ACK retry keeps operation key, prevents double click and reads original zero result', async () => {
  const originalPost = api.post
  const originalError = toast.error
  const originalSuccess = toast.success
  let errors = 0
  let success = ''
  const bodies: unknown[] = []
  toast.error = (() => {
    errors += 1
    return 1
  }) as typeof toast.error
  toast.success = ((message: unknown) => {
    success = String(message)
    return 1
  }) as typeof toast.success
  api.get = (async () => ({
    data: { success: true, data: nativeStatus(false) },
  })) as typeof api.get
  api.post = (async (_url, body) => {
    bodies.push(body)
    if (bodies.length === 1) throw new Error('ACK lost')
    return {
      data: {
        success: true,
        data: {
          already_claimed: false,
          record: {
            ...nativeZeroRecord,
            origin: 'native',
            policy_revision: 1,
            trust_level: 0,
          },
        },
      },
    }
  }) as typeof api.post
  const calendar = await mountNativeCalendar()
  try {
    const button = [
      ...calendar.host.querySelectorAll<HTMLButtonElement>('button'),
    ].find((element) => element.textContent?.trim() === 'Check in now')
    assert.ok(button)
    await act(async () => {
      button.click()
      button.click()
      await waitForCondition(() => errors === 1, 'lost ACK error not shown')
    })
    assert.equal(bodies.length, 1)
    await act(async () => {
      button.click()
      await waitForCondition(() => success !== '', 'retry result not shown')
    })
    assert.match(success, /Check-in successful! Received/)
    assert.equal(bodies.length, 2)
    assert.deepEqual(bodies[0], bodies[1])
    assert.ok(
      bodies[0] && typeof bodies[0] === 'object' && 'operation_key' in bodies[0]
    )
    assert.equal(button.disabled, true)
    assert.match(
      calendar.host
        .querySelector('[aria-current="date"]')
        ?.getAttribute('aria-label') ?? '',
      /已签到/
    )
  } finally {
    await calendar.close()
    api.post = originalPost
    toast.error = originalError
    toast.success = originalSuccess
  }
})
