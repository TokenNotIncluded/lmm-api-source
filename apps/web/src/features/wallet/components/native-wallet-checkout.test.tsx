/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { Window } from 'happy-dom'

const dom = new Window({ url: 'https://wallet.example' })
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
  'getComputedStyle',
  'requestAnimationFrame',
  'cancelAnimationFrame',
  'sessionStorage',
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
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { readNativeCheckout } = await import('../lib/native-checkout')
const { NativeWalletCheckout } = await import('./native-wallet-checkout')
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
const flush = () => new Promise((resolve) => setTimeout(resolve, 10))
function login(id: number, sid: string) {
  useAuthStore.getState().auth.setBundle({
    user: { id, username: `synthetic-${id}`, role: 1 },
    access_token: 'synthetic-session-token',
    token_type: 'Bearer',
    access_expires_at: 9999999999,
    session: {
      sid,
      current: true,
      login_method: 'password',
      ip: null,
      user_agent: null,
      created_at: 1,
      last_active_at: 1,
      expires_at: 9999999999,
    },
  })
}
async function click(label: string) {
  const button = Array.from(document.querySelectorAll('button')).find(
    (node) => node.textContent?.trim() === label
  )
  assert.ok(button, label)
  await act(async () => {
    button.click()
    await flush()
  })
}

test('native checkout reviews exact Core quote, preserves a lost checkout and resumes by read', async () => {
  const oldGet = api.get,
    oldPost = api.post
  dom.sessionStorage.clear()
  login(19, 'checkout-session')
  const container = document.body.appendChild(document.createElement('div'))
  let root = createRoot(container)
  const writes: Array<{ url: string; data: Record<string, string> }> = []
  let coreCredit = false
  const order = {
    order_id: 'top_42',
    state: 'awaiting_payment',
    currency: 'USD',
    amount_minor: '1000',
    credit_units: '5000000',
    credit_unit: 'credit_500k_usd',
    checkout_url: 'https://pay.example/42',
    provider_paid: true,
    core_credit_confirmed: false,
  }
  api.get = (async (url: string) => ({
    data: {
      success: true,
      data:
        url === '/api/user/payments/channels'
          ? {
              available: true,
              channels: [
                {
                  id: '1',
                  provider: 'stripe',
                  environment: 'test',
                  currency: 'USD',
                },
              ],
            }
          : {
              ...order,
              state: coreCredit ? 'credited' : 'credit_pending',
              core_credit_confirmed: coreCredit,
            },
    },
  })) as typeof api.get
  api.post = (async (
    url: string,
    data: Record<string, string>,
    options: { skipAuthRefresh?: boolean }
  ) => {
    assert.equal(options.skipAuthRefresh, true)
    assert.equal(
      readNativeCheckout('[19,"checkout-session"]', dom.sessionStorage)?.request
        .request_key,
      data.request_key,
      'identity is saved before sending'
    )
    writes.push({ url, data: { ...data } })
    if (url.endsWith('/quote')) {
      return {
        data: {
          success: true,
          data: {
            ...data,
            order_id: 'top_42',
            account_id: '19',
            provider: 'stripe',
            environment: 'test',
            credit_units: '5000000',
            credit_unit: 'credit_500k_usd',
          },
        },
      }
    }
    throw Object.assign(new Error('session expired'), {
      isAxiosError: true,
      response: { status: 401 },
    })
  }) as typeof api.post
  const render = async () => {
    await act(async () => {
      root.render(
        <I18nextProvider i18n={i18n}>
          <NativeWalletCheckout />
        </I18nextProvider>
      )
      await flush()
    })
  }
  try {
    await render()
    assert.ok(document.body.textContent?.includes('Test payment'))
    assert.equal(writes.length, 0)
    await click('Review payment')
    assert.equal(writes.length, 1)
    assert.ok(document.body.textContent?.includes('5,000,000 Credits'))
    assert.ok(document.body.textContent?.includes('USD 10.00'))
    await click('Confirm payment')
    assert.equal(writes.length, 2)
    assert.equal(writes[1].data.request_key, writes[0].data.request_key)
    assert.equal(writes[1].data.expected_credit_units, '5000000')
    assert.ok(document.body.textContent?.includes('Restore your session'))
    assert.equal(
      document.body.textContent?.includes('Start another payment'),
      false
    )
    assert.equal(
      readNativeCheckout('[19,"checkout-session"]', dom.sessionStorage)
        ?.submitted,
      true
    )
    await act(async () => root.unmount())
    root = createRoot(container)
    await render()
    assert.equal(writes.length, 2, 'mount must not replay a financial write')
    await click('Refresh payment status')
    assert.ok(
      document.body.textContent?.includes(
        'Payment received. Waiting for wallet credit.'
      )
    )
    assert.equal(
      document.body.textContent?.includes('Funds have reached your wallet.'),
      false
    )
    assert.equal(
      document.body.textContent?.includes('Start another payment'),
      false
    )
    coreCredit = true
    await click('Refresh payment status')
    assert.ok(
      document.body.textContent?.includes('Funds have reached your wallet.')
    )
    assert.ok(document.body.textContent?.includes('Start another payment'))
    assert.equal(writes.length, 2)
    await act(async () => {
      login(20, 'another-session')
      await flush()
    })
    assert.equal(document.body.textContent?.includes('top_42'), false)
    assert.equal(writes.length, 2)
    assert.equal(
      readNativeCheckout('[20,"another-session"]', dom.sessionStorage),
      null
    )
  } finally {
    await act(async () => root.unmount())
    container.remove()
    api.get = oldGet
    api.post = oldPost
  }
})

test('Creem fixed amount is read-only and a missing quote is recovered with its original key', async () => {
  const oldGet = api.get,
    oldPost = api.post
  dom.sessionStorage.clear()
  login(21, 'creem-session')
  const container = document.body.appendChild(document.createElement('div'))
  const root = createRoot(container)
  const requests: Array<Record<string, string>> = []
  api.get = (async () => ({
    data: {
      success: true,
      data: {
        available: true,
        channels: [
          {
            id: '3',
            provider: 'creem',
            environment: 'live',
            currency: 'USD',
            product_id: 'product_3',
            fixed_amount_minor: '1499',
          },
        ],
      },
    },
  })) as typeof api.get
  api.post = (async (url: string, data: Record<string, string>) => {
    assert.ok(url.endsWith('/quote'))
    requests.push({ ...data })
    if (requests.length === 1) throw new Error('lost quote response')
    return {
      data: {
        success: true,
        data: {
          ...data,
          order_id: 'top_73',
          account_id: '21',
          provider: 'creem',
          environment: 'live',
          credit_units: '7495000',
          credit_unit: 'credit_500k_usd',
        },
      },
    }
  }) as typeof api.post
  try {
    await act(async () => {
      root.render(
        <I18nextProvider i18n={i18n}>
          <NativeWalletCheckout />
        </I18nextProvider>
      )
      await flush()
    })
    const input = document.getElementById(
      'native-payment-amount'
    ) as HTMLInputElement
    assert.equal(input.readOnly, true)
    assert.equal(input.value, '14.99')
    await click('Review payment')
    assert.equal(requests[0].amount_minor, '1499')
    assert.equal(
      document.body.textContent?.includes('Edit payment amount'),
      false
    )
    await click('Recover the same quote')
    assert.deepEqual(requests[1], requests[0])
    assert.ok(document.body.textContent?.includes('7,495,000 Credits'))
    assert.equal(requests.length, 2)
  } finally {
    await act(async () => root.unmount())
    container.remove()
    api.get = oldGet
    api.post = oldPost
  }
})

test('checkout uses the explicit method and submits every POST field only after a click', async () => {
  const oldGet = api.get,
    oldPost = api.post
  dom.sessionStorage.clear()
  login(22, 'epay-session')
  const container = document.body.appendChild(document.createElement('div'))
  const root = createRoot(container)
  const endpoint = 'https://credit.linux.do/epay/pay/submit.php'
  const fields: Array<[string, string]> = [
    ['pid', '001'],
    ['type', 'epay'],
    ['out_trade_no', 'top_91'],
    ['name', 'Wallet 充值 & Credits +'],
    ['money', '10.00'],
    ['notify_url', 'https://wallet.example/callbacks/2?a=1&b=2'],
    ['return_url', 'https://wallet.example/wallet?order=top_91'],
    ['sign', 'a+b/=='],
    ['sign_type', 'MD5'],
    ['device', 'pc'],
    ['device', 'second'],
    ['optional', ''],
  ]
  const checkoutURL = `${endpoint}?${new URLSearchParams(fields)}`
  let method: 'GET' | 'POST' = 'GET'
  let submissions = 0
  let writes = 0
  container.addEventListener('submit', (event) => {
    if ((event.target as HTMLFormElement).method === 'post') {
      event.preventDefault()
      submissions++
    }
  })
  const order = () => ({
    order_id: 'top_91',
    state: 'awaiting_payment',
    currency: 'LDC',
    amount_minor: '1000',
    credit_units: '5000000',
    credit_unit: 'credit_500k_usd',
    checkout_url: checkoutURL,
    checkout_method: method,
    provider_paid: false,
    core_credit_confirmed: false,
  })
  api.get = (async (url: string) => ({
    data: {
      success: true,
      data:
        url === '/api/user/payments/channels'
          ? {
              available: true,
              channels: [
                {
                  id: '2',
                  provider: 'epay',
                  environment: 'live',
                  currency: 'LDC',
                  payment_type: 'epay',
                },
              ],
            }
          : order(),
    },
  })) as typeof api.get
  api.post = (async (url: string, data: Record<string, string>) => {
    writes++
    return {
      data: {
        success: true,
        data: url.endsWith('/quote')
          ? {
              ...data,
              order_id: 'top_91',
              account_id: '22',
              provider: 'epay',
              environment: 'live',
              credit_units: '5000000',
              credit_unit: 'credit_500k_usd',
            }
          : order(),
      },
    }
  }) as typeof api.post
  try {
    await act(async () => {
      root.render(
        <I18nextProvider i18n={i18n}>
          <NativeWalletCheckout />
        </I18nextProvider>
      )
      await flush()
    })
    await click('Review payment')
    await click('Confirm payment')
    const link = container.querySelector<HTMLAnchorElement>('a')
    assert.equal(link?.href, checkoutURL)
    assert.equal(link?.target, '_blank')
    assert.equal(container.querySelector('form'), null)
    method = 'POST'
    await click('Refresh payment status')
    const form = container.querySelector<HTMLFormElement>('form')
    assert.ok(form)
    assert.equal(container.querySelector('a'), null)
    assert.equal(form.method, 'post')
    assert.equal(form.action, endpoint)
    assert.equal(form.target, '_blank')
    assert.equal(form.getAttribute('rel'), 'noopener noreferrer')
    assert.equal(form.enctype, 'application/x-www-form-urlencoded')
    assert.deepEqual(
      Array.from(
        form.querySelectorAll<HTMLInputElement>('input[type="hidden"]')
      ).map((input) => [input.name, input.value]),
      fields
    )
    assert.equal(submissions, 0)
    await click('Open payment page')
    assert.equal(submissions, 1)
    assert.equal(writes, 2)
  } finally {
    await act(async () => root.unmount())
    container.remove()
    api.get = oldGet
    api.post = oldPost
  }
})

test('Coinbase shows the USDC payment before confirming the original USD quote', async () => {
  const oldGet = api.get,
    oldPost = api.post
  dom.sessionStorage.clear()
  login(23, 'coinbase-session')
  const container = document.body.appendChild(document.createElement('div'))
  const root = createRoot(container)
  const requests: Array<{ url: string; data: Record<string, string> }> = []
  const order = {
    order_id: 'top_98',
    state: 'awaiting_payment',
    currency: 'USD',
    amount_minor: '1000',
    credit_units: '5000000',
    credit_unit: 'credit_500k_usd',
    checkout_url: 'https://business.coinbase.com/checkout/synthetic',
    provider_paid: false,
    core_credit_confirmed: false,
  }
  api.get = (async () => ({
    data: {
      success: true,
      data: {
        available: true,
        channels: [
          {
            id: '8',
            provider: 'coinbase',
            environment: 'test',
            currency: 'USD',
          },
        ],
      },
    },
  })) as typeof api.get
  api.post = (async (url: string, data: Record<string, string>) => {
    requests.push({ url, data: { ...data } })
    return {
      data: {
        success: true,
        data: url.endsWith('/quote')
          ? {
              ...data,
              order_id: order.order_id,
              account_id: '23',
              provider: 'coinbase',
              environment: 'test',
              credit_units: order.credit_units,
              credit_unit: order.credit_unit,
            }
          : order,
      },
    }
  }) as typeof api.post
  try {
    await act(async () => {
      root.render(
        <I18nextProvider i18n={i18n}>
          <NativeWalletCheckout />
        </I18nextProvider>
      )
      await flush()
    })
    assert.ok(container.textContent?.includes('Coinbase · USDC'))
    assert.ok(
      container.textContent?.includes(
        'Pay with USDC through Coinbase. This top-up uses 1 USDC = 1 USD.'
      )
    )
    assert.equal(requests.length, 0)
    await click('Review payment')
    assert.equal(requests.length, 1)
    assert.equal(requests[0].data.currency, 'USD')
    assert.equal(requests[0].data.amount_minor, '1000')
    assert.ok(container.textContent?.includes('USDC 10.00'))
    assert.ok(container.textContent?.includes('5,000,000 Credits'))
    assert.equal(container.querySelector('a'), null)
    await click('Confirm payment')
    assert.equal(requests.length, 2)
    assert.equal(requests[1].data.request_key, requests[0].data.request_key)
    assert.equal(requests[1].data.expected_credit_units, '5000000')
    assert.equal(container.querySelector('a')?.href, order.checkout_url)
    assert.equal(
      container.textContent?.includes('Funds have reached your wallet.'),
      false
    )
  } finally {
    await act(async () => root.unmount())
    container.remove()
    api.get = oldGet
    api.post = oldPost
  }
})
