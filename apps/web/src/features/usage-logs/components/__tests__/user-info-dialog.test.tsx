/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
// @ts-expect-error Bun's test module is only available in the test runtime.
import { afterAll, afterEach, expect, test } from 'bun:test'

import { Window } from 'happy-dom'

const dom = new Window({
  url: 'https://console.example.invalid/usage-logs/common',
})
for (const key of [
  'window',
  'document',
  'navigator',
  'localStorage',
  'HTMLElement',
  'customElements',
  'HTMLInputElement',
  'HTMLButtonElement',
  'SVGElement',
  'Node',
  'Element',
  'Event',
  'MouseEvent',
  'PointerEvent',
  'FocusEvent',
  'CustomEvent',
  'MutationObserver',
  'ResizeObserver',
  'requestAnimationFrame',
  'cancelAnimationFrame',
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

const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const {
  createMemoryHistory,
  createRootRoute,
  createRouter,
  RouterContextProvider,
} = await import('@tanstack/react-router')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { nativeUserFixture, nativeUsageFixture } =
  await import('@/features/native-users/test-fixtures')
const { nativeUserStatisticsFixture } =
  await import('@/features/native-users/detail-test-fixtures')
const { nativeQueryFromSearch } = await import('@/features/native-users/search')
const { UserInfoDialog } = await import('../dialogs/user-info-dialog')

const originalAdapter = api.defaults.adapter
const previousAuth = useAuthStore.getState().auth
const i18n = createInstance()
await i18n.use(initReactI18next).init({
  lng: 'en',
  fallbackLng: 'en',
  resources: { en: { translation: {} } },
})
const router = createRouter({
  routeTree: createRootRoute(),
  history: createMemoryHistory({ initialEntries: ['/'] }),
})
const pause = () => new Promise((resolve) => setTimeout(resolve, 20))
let mounted:
  | { container: HTMLDivElement; root: ReturnType<typeof createRoot> }
  | undefined

function login(role = 10, sid = 'synthetic-detail-dialog') {
  useAuthStore.getState().auth.setBundle({
    access_token: 'synthetic-detail-dialog-credential',
    token_type: 'Bearer',
    access_expires_at: 2000000000,
    user: { id: 901, role, username: 'synthetic-admin' },
    session: {
      sid,
      current: true,
      login_method: 'password',
      ip: '127.0.0.1',
      user_agent: 'synthetic',
      created_at: 1,
      last_active_at: 1,
      expires_at: 2000000000,
    },
  })
}

function user(id: string) {
  const value = nativeUserFixture({ id, username: `detail-user-${id}` })
  if (value.wallet.status === 'available') value.wallet.provenance.user_id = id
  return value
}

async function render(userId: number, open = true) {
  if (!mounted) {
    const container = document.createElement('div')
    document.body.append(container)
    mounted = { container, root: createRoot(container) }
  }
  await act(async () => {
    const instance = mounted
    if (!instance) throw new Error('Expected mounted dialog root')
    instance.root.render(
      <RouterContextProvider router={router}>
        <I18nextProvider i18n={i18n}>
          <UserInfoDialog userId={userId} open={open} onOpenChange={() => {}} />
        </I18nextProvider>
      </RouterContextProvider>
    )
    await pause()
  })
  await act(async () => {
    await pause()
  })
}

function dialog() {
  const element = document.querySelector<HTMLElement>('[role="dialog"]')
  if (!element) throw new Error('Expected user details dialog')
  return element
}

function button(label: string) {
  const element = Array.from(
    dialog().querySelectorAll<HTMLButtonElement>('button')
  ).find((item) => item.textContent === label)
  if (!element) throw new Error(`Missing button: ${label}`)
  return element
}

afterEach(async () => {
  await act(async () => {
    mounted?.root.unmount()
  })
  mounted?.container.remove()
  mounted = undefined
  api.defaults.adapter = originalAdapter
  useAuthStore.setState({ auth: previousAuth })
  document.body.replaceChildren()
})
afterAll(() => dom.close())

test('usage user details show the real native record and exact credits with a management link', async () => {
  login()
  const seen: string[] = []
  api.defaults.adapter = async (config) => {
    seen.push(config.url ?? '')
    return {
      config,
      data: { version: 1, usage: nativeUsageFixture(), user: user('10') },
      headers: {},
      status: 200,
      statusText: 'OK',
    }
  }
  await render(10)
  const content = dialog().textContent ?? ''
  for (const value of [
    'detail-user-10',
    'synthetic-native@example.invalid',
    'Spendable wallet',
    'Economic position',
    '-7',
    'Reserved funds',
    'Debt magnitude',
    'Empty value',
  ]) {
    expect(content).toContain(value)
  }
  expect(content).not.toMatch(/USD|Used Quota|Invitation Code/)
  const link = dialog().querySelector<HTMLAnchorElement>('a')
  if (!link) throw new Error('Expected user management link')
  expect(new URL(link.href).pathname).toBe('/users')
  expect(nativeQueryFromSearch(new URL(link.href).search)?.keyword).toBe('10')
  expect(seen).toEqual(['/core/v1/admin/users/10'])
})

test('changing users clears old data and discards a late response for the previous target', async () => {
  login()
  let release: (() => void) | undefined
  api.defaults.adapter = async (config) => {
    const id = config.url?.split('/').pop()
    if (!id) throw new Error('Expected user detail request path')
    if (id === '11') {
      await new Promise<void>((resolve) => {
        release = resolve
      })
    }
    return {
      config,
      data: { version: 1, usage: nativeUsageFixture(), user: user(id) },
      headers: {},
      status: 200,
      statusText: 'OK',
    }
  }
  await render(10)
  expect(dialog().textContent).toContain('detail-user-10')
  await render(11)
  expect(dialog().textContent).not.toContain('detail-user-10')
  expect(dialog().querySelector('[role="status"]')).not.toBeNull()
  await render(12)
  expect(dialog().textContent).toContain('detail-user-12')
  await act(async () => {
    release?.()
    await pause()
  })
  expect(dialog().textContent).toContain('detail-user-12')
  expect(dialog().textContent).not.toContain('detail-user-11')
})

test('failed refreshes hide prior data and retry explicitly without exposing transport messages', async () => {
  login()
  let calls = 0
  api.defaults.adapter = async (config) => {
    calls++
    if (calls === 2) throw new Error('synthetic private internal diagnostic')
    return {
      config,
      data: { version: 1, usage: nativeUsageFixture(), user: user('10') },
      headers: {},
      status: 200,
      statusText: 'OK',
    }
  }
  await render(10)
  await act(async () => {
    button('Refresh').click()
    await pause()
  })
  expect(dialog().querySelector('[role="alert"]')).not.toBeNull()
  expect(dialog().textContent).not.toContain('detail-user-10')
  expect(dialog().textContent).not.toContain('private internal diagnostic')
  await act(async () => {
    button('Retry').click()
    await pause()
  })
  expect(dialog().textContent).toContain('detail-user-10')
  expect(calls).toBe(3)
})

test('a same-session role change removes private detail and its pending response', async () => {
  login()
  let release: (() => void) | undefined
  api.defaults.adapter = async (config) => {
    await new Promise<void>((resolve) => {
      release = resolve
    })
    return {
      config,
      data: { version: 1, usage: nativeUsageFixture(), user: user('10') },
      headers: {},
      status: 200,
      statusText: 'OK',
    }
  }
  await render(10)
  await act(async () => {
    login(1)
    await pause()
  })
  await act(async () => {
    release?.()
    await pause()
  })
  expect(dialog().textContent).not.toContain('detail-user-10')
  expect(dialog().textContent).toContain('Administrator access')
  expect(dialog().querySelector('a')).toBeNull()
})

test('unavailable wallet data is not displayed as a zero balance', async () => {
  login()
  const unavailable = {
    ...user('10'),
    wallet: { status: 'unavailable', code: 'WALLET_PROJECTION_UNAVAILABLE' },
  }
  api.defaults.adapter = async (config) => ({
    config,
    data: { version: 1, usage: nativeUsageFixture(), user: unavailable },
    headers: {},
    status: 200,
    statusText: 'OK',
  })
  await render(10)
  const wallet = dialog().querySelector(
    'section[aria-label="Wallet (Core credits)"]'
  )
  expect(wallet?.textContent).toContain('Unavailable')
  expect(wallet?.querySelector('dd')).toBeNull()
})

test('native usage keeps large totals exact and separates the personal wallet from other payers', async () => {
  login()
  const usage = nativeUsageFixture({
    gross_settled_units: '9007199254741193',
    net_consumption_units: '9007199254740993',
    personal_wallet_consumption_units: '9007199254740693',
  })
  api.defaults.adapter = async (config) => ({
    config,
    data: { version: 1, user: user('10'), usage },
    headers: {},
    status: 200,
    statusText: 'OK',
  })
  await render(10)
  const section = dialog().querySelector(
    'section[data-testid="personal-user-usage"]'
  )
  expect(section?.textContent).toContain('9,007,199,254,740,993')
  expect(section?.textContent).toContain('Settled requests')
  expect(section?.textContent).toContain('Pending settlement')
  expect(section?.textContent).toContain('Imported history')
  const breakdown = section?.querySelector('details')
  expect(breakdown?.open).toBe(false)
  await act(async () => {
    breakdown?.querySelector('summary')?.click()
    await pause()
  })
  expect(breakdown?.open).toBe(true)
  expect(breakdown?.textContent).toContain('Personal wallet consumption')
  expect(breakdown?.textContent).toContain('9,007,199,254,740,693')
  expect(breakdown?.textContent).toContain('Team wallet consumption')
  expect(breakdown?.textContent).toContain('Subscription consumption')
  expect(breakdown?.textContent).toContain('Sponsored consumption')
  expect(breakdown?.textContent).toContain('Refunded consumption')
})

test('unavailable usage keeps the profile visible without invented zero statistics', async () => {
  login()
  api.defaults.adapter = async (config) => ({
    config,
    data: {
      version: 1,
      user: user('10'),
      usage: { status: 'unavailable', code: 'USAGE_PROJECTION_UNAVAILABLE' },
    },
    headers: {},
    status: 200,
    statusText: 'OK',
  })
  await render(10)
  expect(dialog().textContent).toContain('detail-user-10')
  const section = dialog().querySelector(
    'section[data-testid="personal-user-usage"]'
  )
  expect(section?.textContent).toContain('Unavailable')
  expect(section?.querySelector('dd')).toBeNull()
})

test('available zero usage is displayed as zero rather than unavailable', async () => {
  login()
  const usage = nativeUsageFixture({
    settled_requests: '0',
    pending_requests: '0',
    gross_settled_units: '0',
    refunded_units: '0',
    net_consumption_units: '0',
    personal_wallet_consumption_units: '0',
    team_wallet_consumption_units: '0',
    subscription_consumption_units: '0',
    sponsored_consumption_units: '0',
  })
  api.defaults.adapter = async (config) => ({
    config,
    data: { version: 1, user: user('10'), usage },
    headers: {},
    status: 200,
    statusText: 'OK',
  })
  await render(10)
  const section = dialog().querySelector(
    'section[data-testid="personal-user-usage"]'
  )
  expect(section?.textContent).not.toContain('Unavailable')
  expect(
    Array.from(section?.querySelectorAll('dd') ?? [])
      .slice(0, 3)
      .map((value) => value.textContent?.trim())
  ).toEqual(['0Core credits', '0', '0'])
})

test('usage, invitations and original counters keep their exact values and separate units', async () => {
  login()
  const target = user('10')
  const statistics = nativeUserStatisticsFixture(target)
  if (statistics.usage.status !== 'available') {
    throw new Error('Expected usage fixture')
  }
  statistics.usage.tokens = '340282366920938463463374607431768211455'
  statistics.referrals.affiliate_balance_units = '-125000'
  api.defaults.adapter = async (config) => ({
    config,
    data: { version: 1, user: target, usage: nativeUsageFixture(), statistics },
    headers: {},
    status: 200,
    statusText: 'OK',
  })
  await render(10)
  const usage = dialog().querySelector(
    'section[data-testid="actor-user-statistics"]'
  )
  expect(usage?.textContent).toContain('1,234')
  expect(usage?.textContent).toContain(
    '340,282,366,920,938,463,463,374,607,431,768,211,455'
  )
  expect(usage?.textContent).toContain('250,000')
  expect(usage?.textContent).toContain('overlap personal account usage')
  const personal = dialog().querySelector(
    'section[data-testid="personal-user-usage"]'
  )
  expect(personal?.textContent).toContain('Personal account usage')
  expect(personal?.textContent).toContain('1,000')
  expect(personal?.textContent).toContain(
    'Imported history and team-owned API keys are excluded.'
  )
  expect(personal?.textContent).not.toContain('250,000')
  expect(usage?.textContent).not.toContain('251,000')
  expect(usage?.textContent).toContain('500,000 normalized credits = 1 USD.')
  const invitations = dialog().querySelector(
    'section[aria-label="Invitation statistics"]'
  )
  expect(invitations?.textContent).toContain('Recorded invitees12')
  expect(invitations?.textContent).toContain('-125,000')
  expect(invitations?.textContent).toContain(
    'Cumulative granted rewards (normalized credits)700,000'
  )
  const historical = dialog().querySelector(
    'details[data-testid="imported-user-counters"]'
  )
  expect(historical?.textContent).toContain(
    'Historical usage (source units)-17'
  )
  expect(historical?.textContent).toContain('Historical request count76')
  expect(historical?.textContent).toContain(
    'Historical rewards (source units)450,000'
  )
  expect(usage?.textContent).not.toContain('450,000')
})

test('unpriced history and missing invitation data remain unavailable, while known zero stays zero', async () => {
  login()
  const target = user('10')
  const statistics = nativeUserStatisticsFixture(target)
  if (statistics.usage.status !== 'available') {
    throw new Error('Expected usage fixture')
  }
  statistics.usage.requests = '0'
  statistics.usage.credits = null
  statistics.usage.pricing_available = false
  statistics.historical_counters = { status: 'not_imported' }
  statistics.referrals.invited_users = null
  statistics.referrals.affiliate_balance_units = null
  statistics.referrals.reviewed_rewards = null
  api.defaults.adapter = async (config) => ({
    config,
    data: { version: 1, user: target, usage: nativeUsageFixture(), statistics },
    headers: {},
    status: 200,
    statusText: 'OK',
  })
  await render(10)
  const usage = dialog().querySelector(
    'section[data-testid="actor-user-statistics"]'
  )
  expect(usage?.textContent).toContain('Requests0')
  expect(usage?.textContent).toContain('Usage (normalized credits)Unavailable')
  expect(usage?.textContent).toContain('Some history has no verified price')
  const invitations = dialog().querySelector(
    'section[aria-label="Invitation statistics"]'
  )
  expect(
    Array.from(invitations?.querySelectorAll('dd') ?? []).map(
      (item) => item.textContent
    )
  ).toEqual(['Unavailable', 'Unavailable', 'Unavailable'])
  expect(
    dialog().querySelector('details[data-testid="imported-user-counters"]')
      ?.textContent
  ).toContain('No imported account counters are available.')
})
