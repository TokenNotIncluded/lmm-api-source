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
const { useAuthStore } = await import('@/stores/auth-store')
const { useWalletCurrencyPreferenceStore } =
  await import('@/stores/wallet-currency-preference-store')
const i18n = createInstance()
await i18n.use(initReactI18next).init({ lng: 'en', resources: {} })
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
  window.history.replaceState({}, '', '/wallet')
})
after(() => domWindow.close())

async function renderWallet(
  activated = false,
  options: {
    setting?: AuthUser['setting']
    quota?: number
    preview?: boolean
    selfQuota?: number
    paymentReturn?: 'return' | 'cancel'
    nativeChannels?: unknown
    nativeResponse?: (url: string, method: string, data: unknown) => unknown
    user?: Partial<AuthUser>
  } = {}
) {
  const user = {
    ...options.user,
    id: 7,
    username: 'checkout-user',
    role: 1,
    developer_access_granted: activated,
    quota: options.quota ?? 0,
    setting: options.setting,
  }
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
    api.defaults.adapter = async (config) => {
      const url = config.url ?? ''
      const body = options.nativeResponse
        ? options.nativeResponse(url, config.method ?? 'get', config.data)
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
    // Only an absent classic route selects the native contract. A service
    // failure must remain visible instead of changing financial protocols.
    if (
      user.wallet_unit === 'core_credit' &&
      typeof url === 'string' &&
      url.startsWith('/api/subscription/')
    ) {
      throw new AxiosError('Route not found', undefined, undefined, undefined, {
        status: 404,
        statusText: 'Not Found',
        data: { error: 'not_found' },
        headers: {},
        config: { url } as InternalAxiosRequestConfig,
      })
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
  return { container, queryClient }
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
  for (const label of ['Transfer']) {
    const button = [
      ...container.querySelectorAll<HTMLButtonElement>('button'),
    ].find((item) => item.textContent?.trim() === label)
    assert.ok(button?.disabled, label)
    await act(async () => button.click())
  }
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
    /Credit conversion unavailable: USD \/ CNY — Unknown/
  )
  assert.equal(posts.length, 0)
  assert.equal(
    [...container.querySelectorAll('button')].some(
      (b) => b.textContent?.trim() === 'Subscribe Now'
    ),
    false
  )
  assert.ok(container.querySelector('#native-redeem-code'))
  assert.equal(button('Transfer').disabled, true)
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

test('native workspaces fail closed when their actual catalogs are unavailable', async () => {
  const requests: string[] = []
  const { container, queryClient } = await renderWallet(true, {
    quota: 500000,
    user: {
      financial_projection_available: true,
      wallet_unit: 'core_credit',
      wallet_money_basis: 'unknown',
      wallet_money_available: false,
    },
    nativeResponse: (url, method) => {
      requests.push(method)
      throw new Error(`Unavailable module: ${url}`)
    },
  })
  assert.match(container.textContent ?? '', /Request failed|Failed to load/)
  assert.equal(
    [...container.querySelectorAll('button')].some(
      (b) =>
        b.textContent === 'Preview purchase' ||
        b.textContent === 'Subscribe Now'
    ),
    false
  )
  assert.ok(requests.length > 0)
  assert.ok(requests.every((method) => method === 'get'))
  queryClient.clear()
})

test('L0 wallet preview preserves the signed-in account and mounts no financial clients', async () => {
  const requests: string[] = []
  const { container } = await renderWallet(true, {
    preview: true,
    user: { wallet_unit: 'core_credit', financial_projection_available: true },
    nativeResponse: (url) => {
      requests.push(url)
      throw new Error('Preview must not request financial data')
    },
  })
  assert.ok(container.textContent?.includes('Your next idea starts here'))
  assert.equal(requests.length, 0)
  assert.equal(
    useAuthStore.getState().auth.user?.developer_access_granted,
    true
  )
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
