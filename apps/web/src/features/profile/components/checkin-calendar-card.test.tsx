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
import { after, afterEach, beforeEach, describe, test } from 'node:test'

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
const originalPost = api.post
type WidgetOptions = {
  sitekey: string
  action: string
  cData: string
  callback: (token: string) => void
  'expired-callback': () => void
}
let widgets: WidgetOptions[] = []
function widget(index: number) {
  const options = widgets[index]
  assert.ok(options, `Missing Turnstile widget ${index}`)
  return options
}
beforeEach(() => {
  widgets = []
  window.turnstile = {
    render: (_element, options) => {
      widgets.push(options as WidgetOptions)
      return widgets.length
    },
    remove: () => {},
  }
})
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
  api.post = originalPost
  delete window.turnstile
  document.body.replaceChildren()
})

after(() => domWindow.close())

describe('check-in calendar accessibility', () => {
  test('renders localized, non-actionable dates and bounded month navigation', async () => {
    const now = new Date()
    const recordDate = `${now.getFullYear()}-${String(
      now.getMonth() + 1
    ).padStart(2, '0')}-01`
    api.get = (async (url) => ({
      data: {
        success: true,
        data: {
          ...nativeStatus(
            false,
            new URL(String(url), 'https://test.invalid').searchParams.get(
              'month'
            ) ?? recordDate.slice(0, 7)
          ),
          today: `${recordDate.slice(0, 7)}-15`,
          total_checkins: 1,
          total_quota_awarded: '100',
          records:
            !String(url).includes('month=') ||
            String(url).endsWith(recordDate.slice(0, 7))
              ? [
                  {
                    ...nativeZeroRecord,
                    checkin_date: recordDate,
                    quota_awarded: '100',
                  },
                ]
              : [],
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
            <CheckinCalendarCard checkinEnabled />
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
      data: nativeStatus(false),
    },
  })) as typeof api.get
  api.post = (async (url, body) =>
    url === '/api/user/checkin/challenges'
      ? challengeResponse(body)
      : {
          data: {
            success: true,
            data: {
              already_claimed: false,
              record: {
                ...nativeZeroRecord,
                quota_awarded: '3500000',
                origin: 'native',
                journal_id: 1,
                policy_revision: 1,
              },
            },
          },
        }) as typeof api.post
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
            <CheckinCalendarCard checkinEnabled />
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
    })
    assert.equal(widgets.length, 1)
    await act(async () => {
      widget(0).callback('verified-token')
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
      reward_multiplier: 1,
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
function challengeResponse(
  body: unknown,
  overrides: Record<string, unknown> = {}
) {
  return {
    data: {
      success: true,
      data: {
        challenge_id: 'server-challenge',
        operation_key: (body as { operation_key: string }).operation_key,
        day: '2030-01-01',
        expires_at: Math.floor(Date.now() / 1000) + 120,
        turnstile_site_key: 'server-site-key',
        turnstile_action: 'daily_checkin',
        turnstile_cdata: 'server-binding',
        passkey_required: false,
        passkey_options: null,
        ...overrides,
      },
    },
  }
}
function claimResponse(alreadyClaimed = false) {
  return {
    data: {
      success: true,
      data: {
        already_claimed: alreadyClaimed,
        record: {
          ...nativeZeroRecord,
          origin: 'native',
          policy_revision: 1,
        },
      },
    },
  }
}
function findButton(label: string, host: ParentNode = document) {
  const button = [...host.querySelectorAll<HTMLButtonElement>('button')].find(
    (element) => element.textContent?.trim() === label
  )
  assert.ok(button, `Missing button: ${label}`)
  return button
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
          <CheckinCalendarCard checkinEnabled />
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

for (const crossesMidnight of [false, true]) {
  test(`Native lost ACK recovery keeps the original operation${crossesMidnight ? ' across midnight' : ''}`, async () => {
    let serverToday = '2030-01-01'
    const originalSuccess = toast.success
    let success = ''
    const requests: { url: string; body: unknown }[] = []
    toast.success = ((message: unknown) => {
      success = String(message)
      return 1
    }) as typeof toast.success
    api.get = (async () => ({
      data: {
        success: true,
        data: { ...nativeStatus(false), today: serverToday },
      },
    })) as typeof api.get
    api.post = (async (url, body) => {
      requests.push({ url: String(url), body })
      if (url === '/api/user/checkin/challenges') {
        return challengeResponse(body, {
          operation_key: 'original-server-operation',
        })
      }
      if (requests.length === 2) {
        if (crossesMidnight) serverToday = '2030-01-02'
        throw new Error('ACK lost')
      }
      return claimResponse()
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
      })
      assert.equal(requests.length, 1)
      assert.equal(widgets.length, 1)
      await act(async () => {
        widget(0).callback('verified-token')
        widget(0).callback('verified-token')
      })
      assert.equal(requests.length, 2)
      assert.match(
        document.querySelector('[role="alert"]')?.textContent ?? '',
        /result is not confirmed/
      )
      await act(async () => {
        findButton('Refresh verification').click()
        await waitForCondition(() => success !== '', 'retry result not shown')
      })
      assert.match(success, /Check-in successful! Received/)
      assert.equal(requests.length, 3)
      assert.deepEqual(requests[1], {
        url: '/api/user/checkin',
        body: {
          operation_key: 'original-server-operation',
          challenge_id: 'server-challenge',
          turnstile_token: 'verified-token',
        },
      })
      assert.deepEqual(requests[2], {
        url: '/api/user/checkin',
        body: { operation_key: 'original-server-operation' },
      })
      assert.equal(button.disabled, !crossesMidnight)
      assert.match(
        calendar.host
          .querySelector('[aria-current="date"]')
          ?.getAttribute('aria-label') ?? '',
        crossesMidnight ? /2030/ : /已签到/
      )
    } finally {
      await calendar.close()
      toast.success = originalSuccess
    }
  })
}

function installPasskey(
  get: (options: CredentialRequestOptions) => Promise<unknown>
) {
  const descriptors = [
    [window, 'isSecureContext'],
    [window, 'PublicKeyCredential'],
    [navigator, 'credentials'],
  ] as const
  const originals = descriptors.map(([target, key]) =>
    Object.getOwnPropertyDescriptor(target, key)
  )
  Object.defineProperty(window, 'isSecureContext', {
    configurable: true,
    value: true,
  })
  Object.defineProperty(window, 'PublicKeyCredential', {
    configurable: true,
    value: class {},
  })
  Object.defineProperty(navigator, 'credentials', {
    configurable: true,
    value: { get, create: async () => null },
  })
  return () =>
    descriptors.forEach(([target, key], index) => {
      const descriptor = originals[index]
      if (descriptor) Object.defineProperty(target, key, descriptor)
      else Reflect.deleteProperty(target, key)
    })
}

test('an uncredited operation from yesterday is recovered before requesting today with a new key', async () => {
  let serverToday = '2030-01-01'
  const requests: { url: string; body: Record<string, unknown> }[] = []
  api.get = (async () => ({
    data: {
      success: true,
      data: { ...nativeStatus(false), today: serverToday },
    },
  })) as typeof api.get
  api.post = (async (url, body) => {
    requests.push({ url: String(url), body: body as Record<string, unknown> })
    if (requests.length === 1) {
      return challengeResponse(body, {
        operation_key: 'yesterday-operation-key',
      })
    }
    if (requests.length === 2) {
      serverToday = '2030-01-02'
      throw new Error('ACK lost')
    }
    if (requests.length === 3) {
      return {
        data: {
          success: false,
          error: { code: 'CHECKIN_VERIFICATION_REQUIRED' },
        },
      }
    }
    return challengeResponse(body, {
      day: serverToday,
      challenge_id: 'today-challenge',
    })
  }) as typeof api.post
  const calendar = await mountNativeCalendar()
  try {
    await act(async () => findButton('Check in now', calendar.host).click())
    await act(async () => widget(0).callback('yesterday-token'))
    await act(async () => findButton('Refresh verification').click())
    assert.deepEqual(requests[2], {
      url: '/api/user/checkin',
      body: { operation_key: 'yesterday-operation-key' },
    })
    assert.equal(requests[3]?.url, '/api/user/checkin/challenges')
    assert.notEqual(requests[3]?.body.operation_key, 'yesterday-operation-key')
    assert.equal(widgets.length, 2)
  } finally {
    await calendar.close()
  }
})

const testCredential = {
  id: 'credential-id',
  rawId: new Uint8Array([1]).buffer,
  type: 'public-key',
  response: {
    authenticatorData: new Uint8Array([2]).buffer,
    clientDataJSON: new Uint8Array([3]).buffer,
    signature: new Uint8Array([4]).buffer,
    userHandle: null,
  },
}
const strictChallenge = {
  passkey_required: true,
  passkey_options: {
    publicKey: { challenge: 'BQ', userVerification: 'required' },
  },
}

test('server challenge binds Turnstile and requires an explicit Passkey confirmation, with cancellation retry', async () => {
  const requests: { url: string; body: unknown }[] = []
  let passkeyCalls = 0
  const restorePasskey = installPasskey(async (options) => {
    passkeyCalls += 1
    assert.equal(options.publicKey?.userVerification, 'required')
    assert.ok(options.signal)
    if (passkeyCalls === 1) {
      const error = new Error('cancelled')
      error.name = 'NotAllowedError'
      throw error
    }
    return testCredential
  })
  api.get = (async () => ({
    data: { success: true, data: nativeStatus(false) },
  })) as typeof api.get
  api.post = (async (url, body) => {
    requests.push({ url: String(url), body })
    return url === '/api/user/checkin/challenges'
      ? challengeResponse(body, strictChallenge)
      : claimResponse()
  }) as typeof api.post
  const calendar = await mountNativeCalendar()
  try {
    assert.equal(
      requests.length,
      0,
      'opening the calendar must not request a challenge'
    )
    await act(async () => findButton('Check in now', calendar.host).click())
    assert.equal(widgets[0]?.sitekey, 'server-site-key')
    assert.equal(widgets[0]?.action, 'daily_checkin')
    assert.equal(widgets[0]?.cData, 'server-binding')
    const confirm = findButton('Confirm check-in with Passkey')
    assert.equal(confirm.disabled, true)
    await act(async () => widget(0).callback('verified-token'))
    assert.equal(passkeyCalls, 0)
    assert.equal(requests.length, 1)
    assert.equal(confirm.disabled, false)
    assert.equal(widgets.length, 1, 'state updates must not reset Turnstile')
    await act(async () => confirm.click())
    assert.match(
      document.querySelector('[role="alert"]')?.textContent ?? '',
      /cancelled/
    )
    assert.equal(requests.length, 1)
    await act(async () => confirm.click())
    assert.equal(passkeyCalls, 2)
    assert.equal(requests.length, 2)
    const body = requests[1]?.body as Record<string, unknown>
    assert.equal(body.challenge_id, 'server-challenge')
    assert.equal(body.turnstile_token, 'verified-token')
    assert.deepEqual(
      (body.passkey_assertion as Record<string, unknown>).response,
      {
        authenticatorData: 'Ag',
        clientDataJSON: 'Aw',
        signature: 'BA',
        userHandle: null,
      }
    )
  } finally {
    await calendar.close()
    restorePasskey()
  }
})

test('expired verification refresh keeps the server operation and rejects the old widget callback', async () => {
  const bodies: unknown[] = []
  api.get = (async () => ({
    data: { success: true, data: nativeStatus(false) },
  })) as typeof api.get
  api.post = (async (url, body) => {
    assert.equal(url, '/api/user/checkin/challenges')
    bodies.push(body)
    return challengeResponse(body, {
      operation_key: 'restored-operation-key',
      challenge_id: `challenge-${bodies.length}`,
    })
  }) as typeof api.post
  const calendar = await mountNativeCalendar()
  try {
    await act(async () => findButton('Check in now', calendar.host).click())
    await act(async () => widget(0)['expired-callback']())
    await act(async () => widget(0).callback('stale-token'))
    assert.equal(bodies.length, 1)
    await act(async () => findButton('Refresh verification').click())
    assert.deepEqual(bodies[1], { operation_key: 'restored-operation-key' })
    assert.equal(widgets.length, 2)
    await act(async () => widget(0).callback('stale-token'))
    assert.equal(bodies.length, 2)
  } finally {
    await calendar.close()
  }
})

test('pending status recovers a credited operation after a browser reload without a new challenge', async () => {
  const urls: string[] = []
  api.get = (async () => ({
    data: { success: true, data: { ...nativeStatus(false), pending: true } },
  })) as typeof api.get
  api.post = (async (url, body) => {
    urls.push(String(url))
    assert.deepEqual(Object.keys(body as object), ['operation_key'])
    return claimResponse(true)
  }) as typeof api.post
  const calendar = await mountNativeCalendar()
  try {
    await act(async () => findButton('Check in now', calendar.host).click())
    assert.deepEqual(urls, ['/api/user/checkin'])
    assert.equal(widgets.length, 0)
    assert.ok(findButton('已签到', calendar.host).disabled)
  } finally {
    await calendar.close()
  }
})

test('an uncredited pending operation requires a fresh challenge and adopts its original server key', async () => {
  const requests: { url: string; body: unknown }[] = []
  api.get = (async () => ({
    data: { success: true, data: { ...nativeStatus(false), pending: true } },
  })) as typeof api.get
  api.post = (async (url, body) => {
    requests.push({ url: String(url), body })
    if (requests.length === 1) {
      return {
        data: {
          success: false,
          error: { code: 'CHECKIN_VERIFICATION_REQUIRED' },
        },
      }
    }
    if (url === '/api/user/checkin/challenges') {
      return challengeResponse(body, {
        operation_key: 'original-pending-operation',
      })
    }
    return claimResponse()
  }) as typeof api.post
  const calendar = await mountNativeCalendar()
  try {
    await act(async () => findButton('Check in now', calendar.host).click())
    assert.deepEqual(
      requests.map(({ url }) => url),
      ['/api/user/checkin', '/api/user/checkin/challenges']
    )
    await act(async () => widget(0).callback('fresh-token'))
    assert.deepEqual(requests[2], {
      url: '/api/user/checkin',
      body: {
        operation_key: 'original-pending-operation',
        challenge_id: 'server-challenge',
        turnstile_token: 'fresh-token',
      },
    })
  } finally {
    await calendar.close()
  }
})

for (const action of ['unmount', 'account switch'] as const) {
  test(`${action} aborts pending device confirmation and ignores its late assertion`, async () => {
    const previousAuth = useAuthStore.getState().auth
    let closed = false
    let resolveCredential: ((result: unknown) => void) | undefined
    let signal: AbortSignal | undefined
    const restorePasskey = installPasskey((options) => {
      signal = options.signal
      return new Promise((resolve) => {
        resolveCredential = resolve
      })
    })
    let posts = 0
    api.get = (async () => ({
      data: { success: true, data: nativeStatus(false) },
    })) as typeof api.get
    api.post = (async (_url, body) => {
      posts += 1
      return challengeResponse(body, strictChallenge)
    }) as typeof api.post
    const calendar = await mountNativeCalendar()
    try {
      await act(async () => findButton('Check in now', calendar.host).click())
      await act(async () => widget(0).callback('verified-token'))
      await act(async () => findButton('Confirm check-in with Passkey').click())
      assert.equal(signal?.aborted, false)
      if (action === 'unmount') {
        await calendar.close()
        closed = true
      } else {
        await act(async () =>
          useAuthStore
            .getState()
            .auth.setUser({ id: 42, username: 'other', role: 1 })
        )
      }
      assert.equal(signal?.aborted, true)
      await act(async () => resolveCredential?.(testCredential))
      assert.equal(posts, 1)
    } finally {
      if (!closed) await calendar.close()
      restorePasskey()
      useAuthStore.setState({ auth: previousAuth })
    }
  })
}
