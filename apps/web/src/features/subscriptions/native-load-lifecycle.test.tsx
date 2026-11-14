/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import type { InternalAxiosRequestConfig } from 'axios'
import { Window } from 'happy-dom'
import type { ReactNode } from 'react'

import { installSupportedAuthBrowser } from '@/test-support/supported-auth-browser'

const dom = new Window({ url: 'https://synthetic.example/subscriptions' })
dom.document.write('<!doctype html><html><head></head><body></body></html>')
const originals = new Map<string, PropertyDescriptor | undefined>()
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLInputElement',
  'HTMLButtonElement',
  'HTMLSelectElement',
  'SVGElement',
  'Node',
  'NodeFilter',
  'Element',
  'Event',
  'MouseEvent',
  'CustomEvent',
  'MutationObserver',
  'ResizeObserver',
  'DocumentFragment',
  'ShadowRoot',
  'getComputedStyle',
  'requestAnimationFrame',
  'cancelAnimationFrame',
  'sessionStorage',
] as const) {
  originals.set(key, Object.getOwnPropertyDescriptor(globalThis, key))
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: dom[key],
  })
}
originals.set(
  'IS_REACT_ACT_ENVIRONMENT',
  Object.getOwnPropertyDescriptor(globalThis, 'IS_REACT_ACT_ENVIRONMENT')
)
Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
  configurable: true,
  value: true,
})
installSupportedAuthBrowser()
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { NativeWalletSubscriptions } = await import('./native-wallet')
const { NativeSubscriptionAdmin } = await import('./native-admin')
const { NativeExternalQuote } = await import('./native-external-quote')
const { toIntlLocale } = await import('@/i18n/languages')
const i18n = createInstance()
await i18n.use(initReactI18next).init({
  lng: 'en',
  fallbackLng: 'en',
  resources: { en: { translation: {} }, fr: { translation: {} } },
})
const originalAdapter = api.defaults.adapter
const originalAuth = useAuthStore.getState().auth
let root: ReturnType<typeof createRoot> | undefined
function login(user = 52, sid = 'load-session') {
  useAuthStore.getState().auth.setBundle({
    user: { id: user, role: 100, username: 'synthetic-root' },
    access_token: 'synthetic-load-only',
    token_type: 'Bearer',
    access_expires_at: 2000000000,
    session: {
      sid,
      current: true,
      login_method: 'password',
      ip: null,
      user_agent: null,
      created_at: 1,
      last_active_at: 1,
      expires_at: 2000000000,
    },
  })
}
function deferred() {
  let resolve!: (value: unknown) => void
  let reject!: (error: Error) => void
  const promise = new Promise<unknown>((accept, refuse) => {
    resolve = accept
    reject = refuse
  })
  return { promise, resolve, reject }
}
function plan(title: string): import('./native-api').NativePlan {
  return {
    id: 4,
    revision: 1,
    archived: false,
    terms: {
      title,
      price_units: '123456',
      duration_seconds: 86400,
      duration_months: 0,
      reset_period: 'custom',
      reset_seconds: 7200,
      limit_units: '654321',
      models: ['*'],
      groups: ['*'],
      max_purchases_per_user: 0,
      enabled: true,
      allow_balance_pay: true,
    },
  }
}
function history(title: string) {
  return [
    {
      id: 8,
      subscription_id: 9,
      plan: plan(title),
      starts_at: 1700000000,
      ends_at: 1700086400,
      used_units: '0',
      reserved_units: '0',
      entitlement_active: true,
      renewal_canceled: false,
      canceled_at: null,
      refund_journal_id: null,
    },
  ]
}
type Kind = 'wallet' | 'admin'
function transport(kind: Kind) {
  const slots = Array.from({ length: 3 }, () => ({
    catalog: deferred(),
    purchases: deferred(),
  }))
  const seen: InternalAxiosRequestConfig[] = []
  let catalogs = 0,
    histories = 0
  api.defaults.adapter = async (config) => {
    seen.push(config)
    assert.equal(
      config.method,
      'get',
      'load must never dispatch a financial write'
    )
    let data: unknown
    if (
      config.url ===
      `/core/v1/subscriptions/${kind === 'admin' ? 'admin/' : ''}plans`
    ) {
      const slot = slots[catalogs++]
      assert.ok(slot)
      data = await slot.catalog.promise
    } else if (
      kind === 'wallet' &&
      config.url === '/core/v1/subscriptions/purchases'
    ) {
      const slot = slots[histories++]
      assert.ok(slot)
      data = await slot.purchases.promise
    } else {
      assert.ok(
        config.url?.endsWith('/epay-prices') ||
          config.url?.endsWith('/pancake-prices'),
        `unexpected read ${config.url}`
      )
      data = { prices: [] }
    }
    return { config, data, headers: {}, status: 200, statusText: 'OK' }
  }
  return {
    slots,
    seen,
    count: () => catalogs,
    settle: async (index: number, title: string, error = false) => {
      const slot = slots[index]
      assert.ok(slot)
      await act(async () => {
        if (error) {
          slot.catalog.reject(new Error('synthetic old load failure'))
        } else {
          slot.catalog.resolve({
            contract: 'core.subscriptions.v1',
            plans: [plan(title)],
          })
        }
        slot.purchases.resolve({
          contract: 'core.subscriptions.v1',
          purchases: history(title),
        })
      })
    },
  }
}
async function mount(children: ReactNode) {
  const host = document.body.appendChild(document.createElement('div'))
  root = createRoot(host)
  await act(async () =>
    root?.render(<I18nextProvider i18n={i18n}>{children}</I18nextProvider>)
  )
  return host
}
function view(kind: Kind) {
  return kind === 'wallet' ? (
    <NativeWalletSubscriptions />
  ) : (
    <NativeSubscriptionAdmin />
  )
}
async function refresh(host: HTMLElement) {
  const button = [...host.querySelectorAll('button')].find(
    (button) => button.textContent?.trim() === 'Refresh'
  )
  assert.ok(button)
  await act(async () => button.click())
}
afterEach(async () => {
  await act(async () => root?.unmount())
  root = undefined
  api.defaults.adapter = originalAdapter
  useAuthStore.setState({ auth: originalAuth })
  document.body.replaceChildren()
  dom.sessionStorage.clear()
  await i18n.changeLanguage('en')
})
after(() => {
  dom.close()
  for (const [key, descriptor] of originals) {
    if (descriptor) Object.defineProperty(globalThis, key, descriptor)
    else Reflect.deleteProperty(globalThis, key)
  }
})
for (const kind of ['wallet', 'admin'] as const) {
  for (const oldError of [false, true]) {
    test(`${kind}: old ${oldError ? 'error' : 'success'} after new success cannot replace the current data or notice`, async () => {
      login()
      const read = transport(kind)
      const host = await mount(view(kind))
      assert.equal(read.count(), 1)
      await refresh(host)
      assert.equal(read.count(), 2)
      await read.settle(1, 'newest catalogue')
      assert.ok(host.textContent?.includes('newest catalogue'))
      await read.settle(0, 'obsolete catalogue', oldError)
      assert.ok(host.textContent?.includes('newest catalogue'))
      assert.ok(!host.textContent?.includes('obsolete catalogue'))
      assert.ok(!host.textContent?.includes('Request failed'))
      assert.equal(read.count(), 2)
      assert.ok(
        read.seen.every(
          (request) =>
            request.authScope?.userId === 52 &&
            request.authScope.sessionId === 'load-session'
        )
      )
    })
  }
  test(`${kind}: an obsolete failure while its replacement is pending cannot clear loading or publish an error`, async () => {
    login()
    const read = transport(kind)
    const host = await mount(view(kind))
    await refresh(host)
    await read.settle(0, 'obsolete', true)
    assert.ok(!host.textContent?.includes('Request failed'))
    if (kind === 'wallet') assert.ok(host.textContent?.includes('Loading...'))
    await read.settle(1, 'replacement completed')
    assert.ok(host.textContent?.includes('replacement completed'))
    if (kind === 'wallet') assert.ok(!host.textContent?.includes('Loading...'))
  })
  for (const boundary of ['account', 'session'] as const) {
    test(`${kind}: ${boundary} reset fences the previous workspace response`, async () => {
      login()
      const read = transport(kind)
      const host = await mount(view(kind))
      await act(async () =>
        login(boundary === 'account' ? 53 : 52, 'next-session')
      )
      assert.equal(read.count(), 2)
      await read.settle(1, 'new owner catalogue')
      await read.settle(0, 'old owner private catalogue')
      assert.ok(host.textContent?.includes('new owner catalogue'))
      assert.ok(!host.textContent?.includes('old owner private catalogue'))
      assert.ok(!host.textContent?.includes('Request failed'))
      assert.deepEqual(
        read.seen
          .filter((request) => request.url?.endsWith('/plans'))
          .map((request) => request.authScope),
        [
          { userId: 52, sessionId: 'load-session' },
          {
            userId: boundary === 'account' ? 53 : 52,
            sessionId: 'next-session',
          },
        ]
      )
    })
  }
  test(`${kind}: unmount invalidates the pending load without any write`, async () => {
    login()
    const read = transport(kind)
    const host = await mount(view(kind))
    await act(async () => root?.unmount())
    root = undefined
    await read.settle(0, 'unmounted private catalogue')
    assert.equal(host.textContent, '')
    assert.equal(read.count(), 1)
    assert.ok(read.seen.every((request) => request.method === 'get'))
  })
}

test('wallet and frozen external quote independently localize dates and exact units without submitting a recovered pending quote', async () => {
  login()
  await i18n.changeLanguage('fr')
  const read = transport('wallet')
  const frozen = plan('frozen quote')
  dom.sessionStorage.setItem(
    'lmm-commerce:52:load-session:subscription',
    JSON.stringify({
      kind: 'purchase',
      confirmed: false,
      request: {
        plan_id: 4,
        renew_from: null,
        operation_key: 'locale-confirmation-key-0001',
      },
      quote: {
        id: 'f'.repeat(64),
        plan: frozen,
        starts_at: 1700000000,
        ends_at: 1700086400,
        expires_at: 2000000000,
        renew_from: null,
      },
    })
  )
  const host = await mount(
    <>
      <div data-wallet-locale>
        <NativeWalletSubscriptions />
      </div>
      <div data-external-locale>
        <NativeExternalQuote
          plan={frozen}
          orderId='private-string-order'
          currency='CNY'
          amountMinor='900719925474099300'
          expiresAt='1700086400'
        />
      </div>
    </>
  )
  await read.settle(0, 'localized catalogue')
  const wallet = host.querySelector('[data-wallet-locale]')
  const quote = host.querySelector('[data-external-locale]')
  assert.ok(wallet && quote)
  const locale = toIntlLocale(i18n.language)
  for (const element of [wallet, quote]) {
    assert.ok(
      element.textContent?.includes(
        BigInt(frozen.terms.limit_units).toLocaleString(locale)
      )
    )
    assert.ok(
      element.textContent?.includes(
        frozen.terms.reset_seconds.toLocaleString(locale)
      )
    )
    assert.ok(
      element.textContent?.includes(
        new Date(1700086400 * 1000).toLocaleString(locale)
      )
    )
  }
  assert.ok(
    wallet.textContent?.includes(
      BigInt(frozen.terms.price_units).toLocaleString(locale)
    )
  )
  assert.ok(
    wallet.textContent?.includes(
      new Date(1700000000 * 1000).toLocaleString(locale)
    )
  )
  assert.ok(wallet.textContent?.includes('Confirm purchase'))
  assert.ok(
    quote.textContent?.includes(
      frozen.terms.duration_seconds.toLocaleString(locale)
    )
  )
  assert.ok(quote.textContent?.includes('CNY 9007199254740993.00'))
  assert.ok(quote.textContent?.includes('private-string-order'))
  const saved = JSON.parse(
    dom.sessionStorage.getItem('lmm-commerce:52:load-session:subscription') ||
      '{}'
  )
  assert.equal(saved.confirmed, false)
  assert.equal(saved.quote.id, 'f'.repeat(64))
  assert.ok(read.seen.every((request) => request.method === 'get'))
})
