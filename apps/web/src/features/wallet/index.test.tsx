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
import { after, afterEach, test } from 'node:test'

import { AxiosError, type InternalAxiosRequestConfig } from 'axios'
import { Window } from 'happy-dom'
import type React from 'react'

import type { AuthUser } from '@/stores/auth-store'

const domWindow = new Window({
  url: 'https://console.example.test/wallet',
})
domWindow.document.write('<!doctype html><html><body></body></html>')
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
  'getComputedStyle',
  'matchMedia',
  'customElements',
  'CSSStyleSheet',
  'localStorage',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: domWindow[key],
  })
}
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
Object.defineProperty(globalThis, 'IntersectionObserver', {
  configurable: true,
  value: class {
    constructor(private callback: IntersectionObserverCallback) {}
    observe(target: Element) {
      this.callback(
        [
          {
            target,
            isIntersecting: true,
            intersectionRatio: 1,
          } as IntersectionObserverEntry,
        ],
        this as unknown as IntersectionObserver
      )
    }
    disconnect() {}
  },
})

const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const {
  createRouter,
  createRootRoute,
  createMemoryHistory,
  RouterContextProvider,
} = await import('@tanstack/react-router')
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { Wallet } = await import('./index')
const { api } = await import('@/lib/api')
const { toast } = await import('sonner')
const { useAuthStore } = await import('@/stores/auth-store')
const { useWalletCurrencyPreferenceStore } =
  await import('@/stores/wallet-currency-preference-store')
const i18n = createInstance()
await i18n.use(initReactI18next).init({ lng: 'en', resources: {} })
const { useSystemConfigStore } = await import('@/stores/system-config-store')
const originalConfig = useSystemConfigStore.getState().config
const originalGet = api.get
const originalPost = api.post
const originalAdapter = api.defaults.adapter
const mounted: Array<{
  root: ReturnType<typeof createRoot>
  container: HTMLDivElement
}> = []
async function render(node: React.ReactNode) {
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  mounted.push({ root, container })
  await act(async () => {
    const router = createRouter({
      routeTree: createRootRoute(),
      history: createMemoryHistory({
        initialEntries: [
          window.location.pathname +
            window.location.search +
            window.location.hash,
        ],
      }),
    })
    root.render(
      <RouterContextProvider router={router}>
        <I18nextProvider i18n={i18n}>{node}</I18nextProvider>
      </RouterContextProvider>
    )
  })
  return container
}

afterEach(async () => {
  for (const { root, container } of mounted.splice(0)) {
    await act(async () => root.unmount())
    container.remove()
  }
  api.get = originalGet
  api.post = originalPost
  api.defaults.adapter = originalAdapter
  window.sessionStorage.clear()
  useAuthStore.getState().auth.reset('complete')
  useSystemConfigStore.setState({ config: originalConfig })
  window.history.replaceState({}, '', '/wallet')
})
after(() => domWindow.close())

async function setSectionOpen(section: HTMLDetailsElement, open: boolean) {
  await act(async () => {
    section.open = open
    section.dispatchEvent(new window.Event('toggle'))
    await new Promise((resolve) => setTimeout(resolve, 15))
  })
}

async function waitForText(element: Element, expected: RegExp) {
  const deadline = Date.now() + 2000
  while (!expected.test(element.textContent ?? '') && Date.now() < deadline) {
    await act(() => new Promise<void>((resolve) => setTimeout(resolve, 10)))
  }
  assert.match(element.textContent ?? '', expected)
}

async function renderWallet(
  activated = false,
  options: {
    setting?: AuthUser['setting']
    quota?: number
    preview?: boolean
    openSections?: boolean
    publicStatus?: () => Promise<Record<string, unknown>>
    pendingSubscription?: unknown
    pendingPancakeSubscription?: unknown
    selfQuota?: number
    paymentReturn?: 'return' | 'cancel'
    nativeChannels?: unknown
    resetVouchers?: unknown[] | Promise<unknown[]>
    unavailableTransferReads?: boolean
    nativeResponse?: (
      url: string,
      method: string,
      data: unknown,
      config: InternalAxiosRequestConfig
    ) => unknown
    user?: Partial<AuthUser>
  } = {}
) {
  const reads: string[] = []
  const user = JSON.parse(
    JSON.stringify({
      ...options.user,
      id: 7,
      username: 'checkout-user',
      role: 1,
      developer_access_granted: activated,
      quota: options.quota ?? 0,
      setting: options.setting,
    })
  )
  const refreshedUser = { ...user, quota: options.selfQuota ?? user.quota }
  useAuthStore.getState().auth.setUser(user)
  if (user.wallet_unit === 'core_credit') {
    useAuthStore.getState().auth.setBundle({
      user,
      access_token: 'synthetic-wallet-session',
      token_type: 'Bearer',
      access_expires_at: 9999999999,
      session: {
        sid: 'native-wallet-scope',
        current: true,
        login_method: 'password',
        ip: null,
        user_agent: null,
        created_at: 1,
        last_active_at: 1,
        expires_at: 9999999999,
      },
    })
    if (options.pendingSubscription) {
      window.sessionStorage.setItem(
        'lmm-commerce:7:native-wallet-scope:subscription',
        JSON.stringify(options.pendingSubscription)
      )
    }
    if (options.pendingPancakeSubscription) {
      window.sessionStorage.setItem(
        'lmm-commerce:7:native-wallet-scope:subscription-pancake',
        JSON.stringify(options.pendingPancakeSubscription)
      )
    }
    api.defaults.adapter = async (config) => {
      const url = config.url ?? ''
      if (config.method === 'get') reads.push(url)
      const body = options.nativeResponse
        ? options.nativeResponse(
            url,
            config.method ?? 'get',
            config.data,
            config
          )
        : url.endsWith('/plans')
          ? { contract: 'core.subscriptions.v1', plans: [] }
          : url.endsWith('/purchases')
            ? { contract: 'core.subscriptions.v1', purchases: [] }
            : url.endsWith('/campaigns')
              ? { contract: 'core.promotions.v1', campaigns: [] }
              : url.endsWith('/history')
                ? { contract: 'core.promotions.v1', claims: [] }
                : (() => {
                    throw new Error(`Unexpected native request: ${url}`)
                  })()
      return { data: body, status: 200, statusText: 'OK', headers: {}, config }
    }
  }
  api.get = (async (url) => {
    reads.push(String(url))
    if (url === '/api/status') {
      return {
        data: { success: true, data: (await options.publicStatus?.()) ?? {} },
      }
    }
    if (url === '/api/site/navigation') {
      return {
        data: {
          success: true,
          message: '',
          data: { value: '{}', revision: 1 },
        },
      }
    }
    if (url === '/api/subscription/self/reset-vouchers') {
      return {
        data: { success: true, data: (await options.resetVouchers) ?? [] },
      }
    }
    // Retired subscription routes and explicitly unavailable transfer reads
    // return a migration error, never a successful empty collection.
    if (
      user.wallet_unit === 'core_credit' &&
      typeof url === 'string' &&
      (url.startsWith('/api/subscription/') ||
        (options.unavailableTransferReads &&
          (url === '/api/wallet/balance' || url === '/api/wallet-transfer')))
    ) {
      throw new AxiosError(
        'Request failed with status code 503',
        undefined,
        undefined,
        undefined,
        {
          status: 503,
          statusText: 'Unavailable',
          data: { error: { code: 'core_migration_incomplete' } },
          headers: {},
          config: { url } as InternalAxiosRequestConfig,
        }
      )
    }
    return {
      data: {
        success: true,
        data:
          url === '/api/user/payments/channels'
            ? (options.nativeChannels ?? { available: false, channels: [] })
            : url === '/api/user/self'
              ? refreshedUser
              : url.startsWith('/api/user/topup/self?')
                ? {
                    items: [],
                    total: 0,
                  }
                : url === '/api/user/aff'
                  ? ''
                  : [],
      },
    }
  }) as typeof api.get
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const container = await render(
    <QueryClientProvider client={queryClient}>
      <Wallet preview={options.preview} paymentReturn={options.paymentReturn} />
    </QueryClientProvider>
  )
  if (options.openSections !== false) {
    for (const id of [
      'subscription-plans',
      'wallet-rewards',
      'referral-program',
    ]) {
      const section = container.querySelector<HTMLDetailsElement>(`#${id}`)
      if (section) await setSectionOpen(section, true)
    }
  }
  return { container, queryClient, reads }
}

test('unavailable ledger facts disable wallet writes without inventing zero amounts or starting quotes', async () => {
  window.history.replaceState({}, '', '/wallet')
  const posts: string[] = []
  api.post = (async (url: string) => {
    posts.push(url)
    throw new Error('Unavailable wallet must not write')
  }) as typeof api.post
  const { container, queryClient } = await renderWallet(true, {
    quota: 500000,
    user: {
      financial_projection_available: false,
      aff_quota: 900000,
      aff_history_quota: 800000,
    },
  })
  assert.match(container.textContent ?? '', /Wallet data is unavailable/)
  assert.equal(container.querySelector('#topup-amount'), null)
  assert.equal(
    container.querySelector('[data-testid^="wallet-token-cloud-"]'),
    null
  )
  for (const label of ['Add funds', 'Subscriptions', 'Transfer']) {
    const button = [
      ...container.querySelectorAll<HTMLButtonElement>('button'),
    ].find((item) => item.textContent?.trim() === label)
    assert.ok(button?.disabled, label)
    await act(async () => button.click())
  }
  assert.doesNotMatch(
    container.querySelector('#wallet-balance')?.textContent ?? '',
    /500,000|0\.00/
  )
  assert.equal(posts.length, 0)
  queryClient.clear()
})

test('unknown historical money retains credits and offers authoritative reward preview', async () => {
  window.history.replaceState({}, '', '/wallet')
  const posts: string[] = []
  api.post = (async (url: string) => {
    posts.push(url)
    throw new Error('No native payment capability')
  }) as typeof api.post
  const { container, queryClient, reads } = await renderWallet(true, {
    quota: 199671,
    unavailableTransferReads: true,
    user: {
      financial_projection_available: true,
      usage_projection_available: false,
      wallet_unit: 'core_credit',
      wallet_money_basis: 'unknown',
      wallet_money_available: false,
      wallet_spendable_quota: 199671,
      wallet_reserved_quota: 280,
      wallet_debt: 0,
      used_quota: undefined,
      request_count: undefined,
      aff_quota: undefined,
      aff_history_quota: undefined,
      aff_count: undefined,
    },
  })
  assert.match(
    container.querySelector('#wallet-balance')?.textContent ?? '',
    /199,671 Credits/
  )
  assert.match(
    container.querySelector('#wallet-balance')?.textContent ?? '',
    /280 Credits/
  )
  assert.equal(container.querySelector('#topup-amount'), null)
  assert.equal(
    [...container.querySelectorAll('button')].some(
      (b) => b.textContent?.trim() === 'Subscribe Now'
    ),
    false
  )
  assert.ok(container.querySelector('#native-redeem-code'))
  const transferButton = [...container.querySelectorAll('button')].find(
    (item) => item.textContent?.trim() === 'Transfer'
  )
  assert.ok(transferButton && !transferButton.disabled)
  await act(async () => {
    transferButton.click()
    await new Promise((resolve) => setTimeout(resolve, 30))
  })
  const createButton = [...document.querySelectorAll('button')].find(
    (item) => item.textContent?.trim() === 'Create transfer link'
  )
  assert.ok(
    createButton?.disabled,
    'the real balance endpoint must establish transfer readiness'
  )
  assert.ok(reads.includes('/api/wallet/balance'))
  assert.ok(reads.includes('/api/wallet-transfer'))
  assert.match(document.body.textContent ?? '', /Failed to load wallet balance/)
  assert.match(
    document.body.textContent ?? '',
    /Failed to load transfer history/
  )
  assert.doesNotMatch(document.body.textContent ?? '', /No transfers yet/)
  const close = [...document.querySelectorAll('button')].find(
    (item) => item.textContent?.trim() === 'Close'
  )
  assert.ok(close)
  await act(async () => close.click())
  const preview = container.querySelector<HTMLAnchorElement>(
    'a[href="/core/v1/affiliate/claim"]'
  )
  assert.equal(preview?.textContent, 'Preview reward transfer')
  // Observe the navigation intent without loading a server or moving funds.
  preview?.addEventListener('click', (event) => event.preventDefault())
  await act(async () => preview?.click())
  assert.equal(container.querySelector('#transfer-amount'), null)
  assert.equal(container.querySelector('#native-payment-amount'), null)
  assert.equal(posts.length, 0)
  queryClient.clear()
})

test('native recharge stays disabled for unavailable or unreadable wallet facts even if a channel exists', async () => {
  const channels = {
    available: true,
    channels: [
      { id: '1', provider: 'stripe', environment: 'test', currency: 'USD' },
    ],
  }
  for (const [financial, quota] of [
    [false, 199671],
    [true, Number.MAX_SAFE_INTEGER + 1],
  ] as const) {
    const posts: string[] = []
    api.post = (async (url: string) => {
      posts.push(url)
      throw new Error('unavailable wallet must not initiate payment')
    }) as typeof api.post
    const { container, queryClient } = await renderWallet(true, {
      quota,
      nativeChannels: channels,
      user: {
        financial_projection_available: financial,
        wallet_unit: 'core_credit',
        wallet_money_basis: 'unknown',
        wallet_money_available: false,
      },
    })
    assert.equal(container.querySelector('#native-payment-amount'), null)
    assert.equal(posts.length, 0)
    queryClient.clear()
    for (const { root, container } of mounted.splice(0)) {
      await act(async () => root.unmount())
      container.remove()
    }
  }
})

test('unknown historical money still requires explicitly confirmed quoted native recharge', async () => {
  window.history.replaceState({}, '', '/wallet')
  window.sessionStorage.clear()
  const posts: Array<{ url: string; data: Record<string, string> }> = []
  api.post = (async (url: string, data: Record<string, string>) => {
    posts.push({ url, data: { ...data } })
    if (url.endsWith('/quote')) {
      return {
        data: {
          success: true,
          data: {
            ...data,
            order_id: 'top_42',
            account_id: '7',
            provider: 'stripe',
            environment: 'test',
            credit_units: '5000000',
            credit_unit: 'credit_500k_usd',
          },
        },
      }
    }
    throw new Error('synthetic confirmation deliberately stops before payment')
  }) as typeof api.post
  const { container, queryClient } = await renderWallet(true, {
    quota: 199671,
    user: {
      financial_projection_available: true,
      usage_projection_available: false,
      wallet_unit: 'core_credit',
      wallet_money_basis: 'unknown',
      wallet_money_available: false,
      wallet_spendable_quota: 199671,
      wallet_reserved_quota: 280,
      wallet_debt: 0,
    },
    nativeChannels: {
      available: true,
      channels: [
        { id: '1', provider: 'stripe', environment: 'test', currency: 'USD' },
      ],
    },
  })
  const button = (label: string) => {
    const value = [
      ...container.querySelectorAll<HTMLButtonElement>('button'),
    ].find((item) => item.textContent?.trim() === label)
    assert.ok(value, label)
    return value
  }
  assert.match(
    container.querySelector('#wallet-balance')?.textContent ?? '',
    /199,671 Credits/
  )
  assert.match(
    container.querySelector('#wallet-balance')?.textContent ?? '',
    /Reference value unavailable/
  )
  assert.equal(posts.length, 0)
  assert.equal(
    [...container.querySelectorAll('button')].some(
      (b) => b.textContent?.trim() === 'Subscribe Now'
    ),
    false
  )
  assert.ok(container.querySelector('#native-redeem-code'))
  assert.equal(button('Transfer').disabled, false)
  const preview = container.querySelector<HTMLAnchorElement>(
    'a[href="/core/v1/affiliate/claim"]'
  )
  assert.equal(preview?.textContent, 'Preview reward transfer')
  assert.equal(container.querySelector('#transfer-amount'), null)
  await act(async () => {
    button('Review payment').click()
    await new Promise((r) => setTimeout(r, 20))
  })
  assert.equal(posts.length, 1)
  assert.match(posts[0].url, /\/quote$/)
  assert.match(container.textContent ?? '', /5,000,000 Credits/)
  assert.match(container.textContent ?? '', /USD 10\.00/)
  await act(async () => {
    button('Confirm payment').click()
    await new Promise((r) => setTimeout(r, 20))
  })
  assert.equal(posts.length, 2)
  assert.equal(posts[1].data.request_key, posts[0].data.request_key)
  assert.equal(posts[1].data.expected_credit_units, '5000000')
  assert.equal(posts[1].data.amount_minor, '1000')
  assert.equal(posts[1].data.currency, 'USD')
  queryClient.clear()
})

test('native wallet exposes credit subscriptions and rewards without fiat conversion or implicit writes', async () => {
  const plan = {
    id: 4,
    revision: 1,
    archived: false,
    terms: {
      title: 'Credit-only plan',
      price_units: '100001',
      duration_seconds: 3600,
      reset_period: 'grant',
      reset_seconds: 0,
      limit_units: '170003',
      models: ['*'],
      groups: ['*'],
      max_purchases_per_user: 0,
      enabled: true,
    },
  }
  const campaigns = (['checkin', 'envelope'] as const).map((kind, i) => ({
    id: String(i + 1).repeat(64),
    revision: 1,
    spec: {
      kind,
      title: `${kind} credits`,
      award_units: '230007',
      budget_units: '920028',
      max_claims: 4,
      max_claims_per_user: 1,
      starts_at: 1,
      ends_at: 9999999999,
      enabled: true,
    },
    spent_units: '0',
    reserved_units: '0',
    claims: 0,
    reserved_claims: 0,
  }))
  const quote = {
    id: 'a'.repeat(64),
    plan,
    starts_at: 1700000000,
    ends_at: 1700003600,
    expires_at: 9999999999,
    renew_from: null,
  }
  for (const preference of ['USD', 'CNY'] as const) {
    useWalletCurrencyPreferenceStore.getState().setPreference(preference)
    const requests: Array<{ url: string; method: string }> = []
    const { container, queryClient } = await renderWallet(true, {
      quota: 500009,
      user: {
        financial_projection_available: true,
        usage_projection_available: true,
        wallet_unit: 'core_credit',
        wallet_money_basis: 'unknown',
        wallet_money_available: false,
        wallet_spendable_quota: 500009,
        wallet_reserved_quota: 0,
        wallet_debt: 0,
        used_quota: 300013,
        normalized_used_quota: 300013,
        request_count: 2,
      },
      nativeResponse: (url, method) => {
        requests.push({ url, method })
        if (url.endsWith('/plans')) {
          return { contract: 'core.subscriptions.v1', plans: [plan] }
        }
        if (url.endsWith('/purchases')) {
          return { contract: 'core.subscriptions.v1', purchases: [] }
        }
        if (url.endsWith('/campaigns')) {
          return { contract: 'core.promotions.v1', campaigns }
        }
        if (url.endsWith('/history')) {
          return { contract: 'core.promotions.v1', claims: [] }
        }
        if (url.endsWith('/quotes')) return quote
        if (url.endsWith('/confirm')) {
          return {
            quote_id: quote.id,
            status: 'posted',
            purchase_id: 1,
            subscription_id: 1,
            journal_id: 1,
            amount_units: '100001',
            rejection: null,
          }
        }
        const campaign = campaigns.find((c) => url.endsWith(`/${c.spec.kind}`))
        if (campaign || url.endsWith('/redemptions')) {
          return {
            id: 'b'.repeat(64),
            campaign_id: campaign?.id ?? 'c'.repeat(64),
            amount_units: '230007',
            journal_id: 2,
            claimed_at: 1700000000,
          }
        }
        throw new Error(`Unexpected native path: ${url}`)
      },
    })
    const click = async (label: string) => {
      const button = [
        ...document.querySelectorAll<HTMLButtonElement>('button'),
      ].find((item) => item.textContent?.trim() === label)
      assert.ok(button && !button.disabled, label)
      await act(async () => {
        button.click()
        await new Promise((r) => setTimeout(r, 30))
      })
    }
    await act(async () => {
      await new Promise((r) => setTimeout(r, 40))
    })
    assert.match(container.textContent ?? '', /Credit-only plan/)
    assert.match(container.textContent ?? '', /100,001 Core credits/)
    assert.match(container.textContent ?? '', /230,007 Core credits/)
    assert.match(
      container.querySelector('#wallet-balance')?.textContent ?? '',
      /300,013 Credits/
    )
    assert.equal(requests.filter((r) => r.method === 'post').length, 0)
    await click('Preview purchase')
    assert.deepEqual(
      requests.filter((r) => r.method === 'post').map((r) => r.url),
      ['/core/v1/subscriptions/quotes']
    )
    assert.equal(requests.filter((r) => r.url.endsWith('/confirm')).length, 0)
    await click('Confirm')
    const corePosts = () =>
      requests
        .filter((r) => r.method === 'post' && r.url.startsWith('/core/v1/'))
        .map((r) => r.url)
    assert.deepEqual(corePosts(), [
      '/core/v1/subscriptions/quotes',
      `/core/v1/subscriptions/quotes/${quote.id}/confirm`,
    ])
    await click('Check In')
    await click('Claim reward')
    const input = container.querySelector<HTMLInputElement>(
      '#native-redeem-code'
    )
    assert.ok(input)
    const setValue = Object.getOwnPropertyDescriptor(
      window.HTMLInputElement.prototype,
      'value'
    )?.set
    assert.ok(setValue)
    await act(async () => {
      setValue.call(input, `lmmr_${'x'.repeat(43)}`)
      input.dispatchEvent(new window.Event('input', { bubbles: true }))
    })
    await click('Redeem')
    assert.match(
      container.textContent ?? '',
      /Reward received · 230,007 Core credits/
    )
    assert.deepEqual(corePosts(), [
      '/core/v1/subscriptions/quotes',
      `/core/v1/subscriptions/quotes/${quote.id}/confirm`,
      `/core/v1/promotions/campaigns/${campaigns[0].id}/checkin`,
      `/core/v1/promotions/campaigns/${campaigns[1].id}/envelope`,
      '/core/v1/promotions/redemptions',
    ])
    assert.doesNotMatch(container.textContent ?? '', /[$¥￥]|(?:USD|CNY)\s+\d/)
    assert.equal(
      useAuthStore.getState().auth.user?.wallet_money_available,
      false
    )
    queryClient.clear()
    for (const { root, container } of mounted.splice(0)) {
      await act(async () => root.unmount())
      container.remove()
    }
    window.sessionStorage.clear()
  }
})

test('native subscription failures stay local and distinct from confirmed empty results, with explicit retry', async () => {
  let failed = true
  let finishVouchers!: (vouchers: unknown[]) => void
  const resetVouchers = new Promise<unknown[]>((resolve) => {
    finishVouchers = resolve
  })
  const beforeToasts = toast.getHistory().length
  const { container, queryClient, reads } = await renderWallet(true, {
    quota: 500000,
    user: {
      financial_projection_available: true,
      wallet_unit: 'core_credit',
      wallet_money_basis: 'unknown',
      wallet_money_available: false,
    },
    openSections: false,
    resetVouchers,
    nativeResponse: (url, method, _data, config) => {
      assert.equal(method, 'get')
      if (url.startsWith('/core/v1/subscriptions/')) {
        if (failed) {
          throw new AxiosError(
            'Request failed with status code 503',
            'ERR_BAD_RESPONSE',
            config,
            undefined,
            {
              config,
              status: 503,
              statusText: 'Unavailable',
              headers: {},
              data: { error: { code: 'SUBSCRIPTION_UNAVAILABLE' } },
            }
          )
        }
        return url.endsWith('/plans')
          ? { contract: 'core.subscriptions.v1', plans: [] }
          : { contract: 'core.subscriptions.v1', purchases: [] }
      }
      if (url.endsWith('/campaigns')) {
        return { contract: 'core.promotions.v1', campaigns: [] }
      }
      if (url.endsWith('/history')) {
        return { contract: 'core.promotions.v1', claims: [] }
      }
      throw new Error(`Unexpected native path: ${url}`)
    },
  })
  const section = container.querySelector<HTMLDetailsElement>(
    '#subscription-plans'
  )
  assert.ok(section)
  assert.equal(
    section.open,
    false,
    'subscriptions do not crowd the top-up workspace'
  )
  assert.ok(
    !reads.some(
      (url) =>
        url.startsWith('/core/v1/subscriptions/') ||
        url.startsWith('/core/v1/promotions/')
    )
  )
  await setSectionOpen(section, true)
  const plans = [...section.querySelectorAll('[data-slot="card-title"]')]
    .find((title) => title.textContent === 'Subscriptions')
    ?.closest('[data-slot="card"]')
  const vouchers = section.querySelector(
    '[aria-labelledby="banked-reset-heading"]'
  )
  assert.ok(plans)
  assert.ok(vouchers)
  const voucherStatus = vouchers.querySelector('#banked-reset-query-status')
  assert.ok(voucherStatus)
  await waitForText(plans, /Unable to load subscriptions/)
  assert.doesNotMatch(
    plans.textContent ?? '',
    /No subscription plans yet|No data|Loading\.\.\./
  )
  assert.match(vouchers.textContent ?? '', /Banked subscription resets/)
  await waitForText(voucherStatus, /Loading\.\.\./)
  assert.doesNotMatch(vouchers.textContent ?? '', /No banked reset vouchers/)
  assert.equal(
    reads.filter((url) => url === '/core/v1/subscriptions/plans').length,
    1
  )
  assert.equal(
    reads.filter((url) => url === '/core/v1/subscriptions/purchases').length,
    1
  )
  assert.ok(
    !reads.some(
      (url) =>
        (url.startsWith('/api/subscription/') &&
          url !== '/api/subscription/self/reset-vouchers') ||
        url === '/api/user/topup/info'
    )
  )
  assert.equal(toast.getHistory().length, beforeToasts)

  const retry = [...plans.querySelectorAll('button')].find(
    (button) => button.textContent?.trim() === 'Retry'
  )
  assert.ok(retry)
  failed = false
  await act(async () => {
    retry.click()
  })
  await waitForText(plans, /No subscription plans yet/)
  assert.match(plans.textContent ?? '', /No data/)
  assert.doesNotMatch(
    plans.textContent ?? '',
    /Unable to load subscriptions|Loading\.\.\./
  )
  assert.equal(
    reads.filter((url) => url === '/core/v1/subscriptions/plans').length,
    2
  )
  assert.equal(
    reads.filter((url) => url === '/core/v1/subscriptions/purchases').length,
    2
  )
  assert.equal(toast.getHistory().length, beforeToasts)
  assert.match(voucherStatus.textContent ?? '', /Loading\.\.\./)
  await act(async () => finishVouchers([]))
  await waitForText(vouchers, /No banked reset vouchers/)
  assert.doesNotMatch(voucherStatus.textContent ?? '', /Loading\.\.\./)
  assert.equal(
    reads.filter((url) => url === '/api/subscription/self/reset-vouchers')
      .length,
    1
  )
  queryClient.clear()
})

test('native wallet uses declared current rates without legacy display flags and keeps the exact credit balance', async () => {
  const { container, queryClient, reads } = await renderWallet(true, {
    publicStatus: async () => ({
      currency_unit: 'credit',
      credit_unit_schema_version: 2,
      credits_per_usd: 500000,
      credits_per_usd_exact: '500000',
      ledger_quota_per_usd: 500000,
      ledger_quota_per_usd_exact: '500000',
      quota_unit: 'LEDGER_QUOTA',
      cny_per_usd: '6.700304',
    }),
    quota: 8065639,
    setting: JSON.stringify({ wallet_display_currency: 'CNY' }),
    openSections: false,
    user: {
      financial_projection_available: true,
      usage_projection_available: true,
      normalized_used_quota: 1250000,
      used_quota: 2000000,
      request_count: 37,
      wallet_unit: 'core_credit',
      wallet_money_basis: 'unknown',
      wallet_money_available: false,
    },
  })
  const balance = container.querySelector('#wallet-balance')
  assert.match(balance?.textContent ?? '', /Reference value/)
  assert.ok(balance)
  await waitForText(balance, /≈ 108\.08 CNY/)
  assert.match(balance?.textContent ?? '', /8,065,639 Credits/)
  assert.equal(
    balance?.querySelector<HTMLDetailsElement>('[data-native-wallet-details]')
      ?.open,
    false
  )
  const account = useAuthStore.getState().auth.user
  assert.ok(account)
  for (const [currency, reference] of [
    ['CNY', '≈ 108.08 CNY'],
    ['USD', '≈ 16.13 USD'],
    ['CREDIT', null],
  ] as const) {
    await act(async () => {
      useAuthStore.getState().auth.setUser({
        ...account,
        setting: JSON.stringify({ wallet_display_currency: currency }),
      })
    })
    const text: string = balance?.textContent ?? ''
    assert.match(text, /8,065,639 Credits/)
    assert.match(text, /Total Usage1,250,000 Credits/)
    assert.match(text, /API Requests37/)
    if (reference) {
      assert.ok(text.includes(reference))
    } else {
      assert.match(text, /Current Balance/)
      assert.doesNotMatch(text, /≈|Reference value unavailable/)
    }
  }
  assert.equal(useAuthStore.getState().auth.user?.quota, 8065639)
  assert.equal(useAuthStore.getState().auth.user?.wallet_money_available, false)
  assert.equal(useAuthStore.getState().auth.user?.wallet_money_basis, 'unknown')
  assert.ok(
    !reads.some(
      (url) =>
        url.startsWith('/core/v1/subscriptions/') ||
        url.startsWith('/core/v1/promotions/')
    )
  )
  const subscriptions = container.querySelector<HTMLDetailsElement>(
    '#subscription-plans'
  )
  assert.ok(subscriptions)
  await setSectionOpen(subscriptions, true)
  assert.ok(reads.includes('/core/v1/subscriptions/plans'))
  assert.ok(reads.includes('/core/v1/subscriptions/purchases'))
  assert.ok(
    !reads.some(
      (url) =>
        (url.startsWith('/api/subscription/') &&
          url !== '/api/subscription/self/reset-vouchers') ||
        url === '/api/user/topup/info'
    )
  )
  assert.ok(container.querySelector('#banked-reset-heading'))
  assert.ok(reads.includes('/api/subscription/self/reset-vouchers'))
  queryClient.clear()
})

test('a saved subscription intent opens its recovery view and survives folding without remount or writes', async () => {
  const pending = {
    kind: 'purchase',
    request: {
      operation_key: 'synthetic-saved-subscription',
      plan_id: 4,
      renew_from: null,
    },
    confirmed: false,
  }
  const { container, queryClient, reads } = await renderWallet(true, {
    quota: 500000,
    openSections: false,
    pendingSubscription: pending,
    user: { financial_projection_available: true, wallet_unit: 'core_credit' },
  })
  const section = container.querySelector<HTMLDetailsElement>(
    '#subscription-plans'
  )
  assert.ok(section?.open)
  assert.match(section.textContent ?? '', /Retry saved request/)
  const initialReads = reads.length
  await setSectionOpen(section, false)
  await setSectionOpen(section, true)
  assert.match(section.textContent ?? '', /Retry saved request/)
  assert.equal(reads.length, initialReads)
  const saved = window.sessionStorage.getItem(
    'lmm-commerce:7:native-wallet-scope:subscription'
  )
  assert.ok(saved)
  assert.deepEqual(JSON.parse(saved), pending)
  queryClient.clear()
})

test('a saved Pancake subscription intent opens recovery and only reads the original order', async () => {
  const { freezePancakeOrder, pancakeOrderSchema } =
    await import('@/features/subscriptions/native-pancake-api')
  const plan = {
    id: 4,
    revision: 1,
    archived: false,
    terms: {
      title: 'Saved Pancake plan',
      price_units: '500000',
      duration_seconds: 3600,
      reset_period: 'grant',
      reset_seconds: 0,
      limit_units: '25000000',
      models: ['*'],
      groups: ['*'],
      max_purchases_per_user: 0,
      enabled: true,
    },
  }
  const orderId = `sub_${'b'.repeat(64)}`
  const order = {
    order: {
      order_id: orderId,
      account_id: '7',
      plan_id: '4',
      plan_revision: '1',
      payment_channel_id: '8',
      currency: 'CNY',
      amount_minor: '670',
      expires_at_unix: '9999999999',
      confirmed_at_unix: '1700000000',
      canceled_at_unix: null,
      proof_id: '',
      subscription_id: null,
      granted_at_unix: null,
      ends_at_unix: null,
      plan_json: Buffer.from(JSON.stringify(plan)).toString('base64'),
    },
    price: {
      plan_id: 4,
      plan_revision: 1,
      channel_id: 8,
      currency: 'CNY',
      amount_minor: '670',
      store_id: 'synthetic-store',
      product_id: 'synthetic-product',
      product_type: 'one_time',
      billing_period: '',
      provider_plan_reference: '',
      return_url: 'https://console.example.test/wallet',
      checkout_region: 'china',
      checkout_language: 'zh-CN',
      tax_category: '',
    },
    merchant: 'synthetic-merchant',
    environment: 'test',
    provider_reference: 'synthetic-pancake-order',
    state: 'pending',
    latest_grant_id: null,
  }
  const pending = {
    provider: 'waffo_pancake',
    operation_key: 'wallet-pancake-recovery',
    plan_id: 4,
    channel_id: 8,
    order_id: orderId,
    frozen: freezePancakeOrder(pancakeOrderSchema.parse(order)),
    confirm_attempted: true,
  }
  const requests: Array<{ url: string; method: string }> = []
  const orderPath = `/api/user/payments/subscriptions/pancake/orders/${orderId}`
  const { container, queryClient } = await renderWallet(true, {
    quota: 500000,
    openSections: false,
    pendingPancakeSubscription: pending,
    user: { financial_projection_available: true, wallet_unit: 'core_credit' },
    nativeResponse: (url, method) => {
      requests.push({ url, method })
      if (method !== 'get') {
        throw new Error('Recovery must not replay a payment')
      }
      if (url === '/core/v1/subscriptions/plans') {
        return { contract: 'core.subscriptions.v1', plans: [] }
      }
      if (url === '/core/v1/subscriptions/purchases') {
        return { contract: 'core.subscriptions.v1', purchases: [] }
      }
      if (url === orderPath) return { order, checkout: null }
      throw new Error(`Unexpected recovery request: ${url}`)
    },
  })
  const section = container.querySelector<HTMLDetailsElement>(
    '#subscription-plans'
  )
  assert.ok(
    section?.open,
    'an external pending payment must expose its recovery view'
  )
  const pancake = section.querySelector('[aria-label="Pancake"]')
  assert.ok(pancake)
  assert.match(pancake.textContent ?? '', /Saved Pancake plan/)
  assert.match(pancake.textContent ?? '', /CNY 6\.70/)
  assert.match(pancake.textContent ?? '', /Retry saved request/)
  assert.doesNotMatch(
    pancake.textContent ?? '',
    /Result unknown|Recovery storage is unavailable/
  )
  assert.equal(requests.filter(({ url }) => url === orderPath).length, 1)
  assert.ok(requests.every(({ method }) => method === 'get'))
  const initialReads = requests.length
  await setSectionOpen(section, false)
  await setSectionOpen(section, true)
  assert.equal(
    requests.length,
    initialReads,
    'folding keeps the existing recovery instance'
  )
  const saved = window.sessionStorage.getItem(
    'lmm-commerce:7:native-wallet-scope:subscription-pancake'
  )
  assert.ok(saved)
  assert.deepEqual(JSON.parse(saved), pending)
  queryClient.clear()
})

test('wallet and private weekly bookmark wait for a live, supported weekly route', async () => {
  const advertised = { backend_capabilities: { weekly_discounts: true } }
  window.localStorage.setItem('status', JSON.stringify(advertised))
  let rejectStatus!: (error: Error) => void
  let response = new Promise<Record<string, unknown>>((_, reject) => {
    rejectStatus = reject
  })
  const { container, queryClient } = await renderWallet(true, {
    user: { wallet_unit: 'core_credit' },
    openSections: false,
    publicStatus: () => response,
  })
  const originalFetch = globalThis.fetch
  const requests: string[] = []
  let send = (async (url, init) => {
    requests.push(`${init?.method} ${url}`)
    return Response.json({
      ok: true,
      current_utc_week: 345600,
      decisions: [],
      next_before_week: null,
    })
  }) as typeof fetch
  // The weekly client captures its transport at module initialization.
  globalThis.fetch = ((url, init) => send(url, init)) as typeof fetch
  try {
    const { WeeklyDiscounts } = await import('@/features/weekly-discounts')
    globalThis.fetch = originalFetch
    const id = 'a'.repeat(64)
    const bookmark = await render(
      <QueryClientProvider client={queryClient}>
        <WeeklyDiscounts selectedID={id} />
      </QueryClientProvider>
    )
    const link = () => container.querySelector('a[href="/weekly-discounts"]')
    await waitForText(bookmark, /Loading…/)
    assert.equal(link(), null)
    assert.deepEqual(requests, [])

    rejectStatus(new Error('Public status unavailable'))
    await waitForText(bookmark, /Could not refresh/)
    assert.equal(link(), null)
    assert.deepEqual(requests, [])
    response = Promise.resolve({})
    await act(async () => {
      bookmark.querySelector<HTMLButtonElement>('button')?.click()
    })
    await waitForText(bookmark, /Capability unavailable/)
    assert.equal(link(), null)
    assert.deepEqual(requests, [])

    response = Promise.resolve(advertised)
    await act(async () => {
      await queryClient.invalidateQueries({ queryKey: ['status'] })
    })
    await waitForText(container, /Weekly discounts/)
    await waitForText(bookmark, /No request this week/)
    assert.ok(link())
    const endpoint = '/api/user/auth/assistant/weekly-discounts'
    assert.deepEqual(requests.sort(), [
      `GET ${endpoint}`,
      `GET ${endpoint}/${id}`,
    ])
  } finally {
    send = originalFetch
    globalThis.fetch = originalFetch
    window.localStorage.removeItem('status')
    queryClient.clear()
  }
})

test('Wallet layout preview preserves the signed-in account and mounts no financial clients', async () => {
  const requests: string[] = []
  const { container } = await renderWallet(true, {
    preview: true,
    user: { wallet_unit: 'core_credit', financial_projection_available: true },
    nativeResponse: (url) => {
      requests.push(url)
      throw new Error('Preview must not request financial data')
    },
  })
  assert.match(
    container.querySelector('#wallet-checkout')?.textContent ?? '',
    /Wallet data is unavailable/
  )
  assert.equal(requests.length, 0)
  assert.equal(useAuthStore.getState().auth.user?.id, 7)
  for (const button of container.querySelectorAll('button')) {
    assert.equal(button.disabled, true, button.textContent ?? '')
  }
})

test('personal billing dialog shows a retryable error instead of an empty history for root', async () => {
  const { BillingHistoryDialog } =
    await import('./components/dialogs/billing-history-dialog')
  useAuthStore
    .getState()
    .auth.setUser({ id: 7, username: 'root-fixture', role: 100 } as AuthUser)
  const urls: string[] = []
  let failed = true
  api.get = (async (url: string) => {
    urls.push(url)
    if (failed) throw new Error('503 core_migration_incomplete')
    return { data: { success: true, data: { items: [], total: 0 } } }
  }) as typeof api.get
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  await render(
    <QueryClientProvider client={queryClient}>
      <BillingHistoryDialog open onOpenChange={() => {}} />
    </QueryClientProvider>
  )
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 30))
  })
  const dialog = document.querySelector('[role="dialog"]')
  assert.ok(dialog)
  assert.match(dialog.textContent ?? '', /Failed to load billing history/)
  assert.doesNotMatch(dialog.textContent ?? '', /No billing records found/)
  assert.doesNotMatch(dialog.textContent ?? '', /Complete Order/)
  const retry = [...dialog.querySelectorAll('button')].find(
    (button) => button.textContent?.trim() === 'Retry'
  )
  assert.ok(retry)
  failed = false
  await act(async () => {
    retry.click()
    await new Promise((resolve) => setTimeout(resolve, 30))
  })
  assert.match(dialog.textContent ?? '', /No billing records found/)
  assert.doesNotMatch(
    dialog.textContent ?? '',
    /Failed to load billing history/
  )
  assert.ok(urls.every((url) => url.startsWith('/api/user/topup/self?')))
  queryClient.clear()
})

test('reset code redemption refreshes the scoped voucher list in the native subscription workspace', async () => {
  const vouchers: unknown[] = []
  const { container, queryClient, reads } = await renderWallet(true, {
    quota: 500000,
    user: {
      financial_projection_available: true,
      wallet_unit: 'core_credit',
      wallet_money_basis: 'unknown',
      wallet_money_available: false,
    },
    resetVouchers: vouchers,
    nativeResponse: (url, method) => {
      if (url.endsWith('/plans')) {
        return { contract: 'core.subscriptions.v1', plans: [] }
      }
      if (url.endsWith('/purchases')) {
        return { contract: 'core.subscriptions.v1', purchases: [] }
      }
      if (url.endsWith('/campaigns')) {
        return { contract: 'core.promotions.v1', campaigns: [] }
      }
      if (url.endsWith('/history')) {
        return { contract: 'core.promotions.v1', claims: [] }
      }
      assert.equal(method, 'post')
      assert.equal(url, '/core/v1/promotions/redemptions')
      vouchers.push({
        id: 81,
        user_id: 7,
        plan_id: 37,
        plan_title: 'Synthetic reset plan',
        operation_id: 'synthetic-reset-operation',
        status: 'available',
        expires_at: 1900000000,
        redeemed_at: 0,
        created_at: 1800000000,
      })
      return {
        reward_type: 'reset_voucher',
        reset_voucher_id: 81,
        reset_plan_id: 37,
        reset_voucher_expires_at: 1900000000,
      }
    },
  })
  const before = reads.filter(
    (url) => url === '/api/subscription/self/reset-vouchers'
  ).length
  const input = container.querySelector<HTMLInputElement>('#native-redeem-code')
  assert.ok(input)
  const setValue = Object.getOwnPropertyDescriptor(
    window.HTMLInputElement.prototype,
    'value'
  )?.set
  assert.ok(setValue)
  await act(async () => {
    setValue.call(input, 'a'.repeat(32))
    input.dispatchEvent(new window.Event('input', { bubbles: true }))
  })
  const form = input.closest('form')
  assert.ok(form)
  await act(async () => {
    form.dispatchEvent(
      new window.Event('submit', { bubbles: true, cancelable: true })
    )
    await new Promise((resolve) => setTimeout(resolve, 70))
  })
  assert.match(
    container.textContent ?? '',
    /Reset voucher received.*#81.*Plan #37/
  )
  assert.ok(container.querySelector('[data-voucher-id="81"]'))
  assert.ok(
    reads.filter((url) => url === '/api/subscription/self/reset-vouchers')
      .length > before
  )
  assert.equal(
    queryClient.getQueryState([
      'subscription-reset-vouchers',
      7,
      'native-wallet-scope',
    ])?.status,
    'success'
  )
  assert.equal(
    queryClient.getQueryData(['subscription-reset-vouchers']),
    undefined
  )
  const status =
    container.querySelector('#wallet-rewards [role="status"]')?.textContent ??
    ''
  assert.ok(!status.includes('Core credits'))
  queryClient.clear()
})
