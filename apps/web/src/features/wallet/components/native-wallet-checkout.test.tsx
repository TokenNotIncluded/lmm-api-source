/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { AxiosError } from 'axios'
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
const { toast } = await import('sonner')
const { useAuthStore } = await import('@/stores/auth-store')
const { useSystemConfigStore } = await import('@/stores/system-config-store')
const { readNativeCheckout, saveNativeCheckout } =
  await import('../lib/native-checkout')
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

async function enterAmount(value: string) {
  const input = document.querySelector('#native-payment-amount')
  assert.ok(input instanceof HTMLInputElement)
  const setter = Object.getOwnPropertyDescriptor(
    HTMLInputElement.prototype,
    'value'
  )?.set
  assert.ok(setter)
  await act(async () => {
    setter.call(input, value)
    input.dispatchEvent(new Event('input', { bubbles: true }))
    await flush()
  })
}

test('unavailable native payment channels cannot start a quote, including after refresh', async () => {
  const oldGet = api.get
  const oldPost = api.post
  dom.sessionStorage.clear()
  login(21, 'unavailable-payment-session')
  const container = document.body.appendChild(document.createElement('div'))
  const root = createRoot(container)
  let reads = 0
  let writes = 0
  api.get = (async (url: string) => {
    assert.equal(url, '/api/user/payments/channels')
    reads++
    return { data: { success: true, data: { available: false, channels: [] } } }
  }) as typeof api.get
  api.post = (async () => {
    writes++
    throw new Error('unavailable payment channels must not write')
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
    assert.ok(
      container.textContent?.includes(
        'No payment channel is available right now.'
      )
    )
    assert.equal(container.querySelector('form'), null)
    assert.equal(container.querySelector('#native-payment-amount'), null)
    assert.equal(writes, 0)
    await click('Refresh')
    assert.equal(reads, 2)
    assert.equal(container.querySelector('form'), null)
    assert.equal(writes, 0)
    assert.equal(
      readNativeCheckout(
        '[21,"unavailable-payment-session"]',
        dom.sessionStorage
      ),
      null
    )
  } finally {
    await act(async () => root.unmount())
    container.remove()
    api.get = oldGet
    api.post = oldPost
  }
})

test('failed channel reads stay local until explicit retry and are not presented as an empty catalog', async () => {
  const oldAdapter = api.defaults.adapter
  dom.sessionStorage.clear()
  login(21, 'failed-payment-session')
  const container = document.body.appendChild(document.createElement('div'))
  const root = createRoot(container)
  let reads = 0
  let failed = true
  const beforeToasts = toast.getHistory().length
  api.defaults.adapter = async (config) => {
    assert.equal(config.url, '/api/user/payments/channels')
    assert.equal(config.method, 'get')
    reads++
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
          data: { success: false, code: 'PAYMENT_UNAVAILABLE' },
        }
      )
    }
    return {
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: {
        success: true,
        data: { available: false, channels: [] },
      },
    }
  }
  try {
    await act(async () => {
      root.render(
        <I18nextProvider i18n={i18n}>
          <NativeWalletCheckout />
        </I18nextProvider>
      )
      await flush()
    })
    assert.match(
      container.querySelector('[role="alert"]')?.textContent ?? '',
      /Unable to load payment methods/
    )
    assert.doesNotMatch(
      container.textContent ?? '',
      /No payment channel is available right now\./
    )
    assert.equal(reads, 1)
    assert.equal(container.querySelector('form'), null)
    assert.equal(toast.getHistory().length, beforeToasts)
    failed = false
    await click('Retry')
    assert.equal(reads, 2)
    assert.equal(container.querySelector('[role="alert"]'), null)
    assert.match(
      container.querySelector('[role="status"]')?.textContent ?? '',
      /No payment channel is available right now\./
    )
    assert.equal(container.querySelector('form'), null)
    assert.equal(toast.getHistory().length, beforeToasts)
  } finally {
    await act(async () => root.unmount())
    container.remove()
    api.defaults.adapter = oldAdapter
  }
})

test('returning from the payment page resumes the same order after a gateway error', async () => {
  const oldGet = api.get
  const oldPost = api.post
  const originalTimer = globalThis.setTimeout
  const originalVisibility = Object.getOwnPropertyDescriptor(document, 'hidden')
  let hidden = false
  Object.defineProperty(document, 'hidden', {
    configurable: true,
    get: () => hidden,
  })
  const walletSession = '[39,"checkout-rolling-update"]'
  dom.sessionStorage.clear()
  login(39, 'checkout-rolling-update')
  const request = {
    channel_id: '1',
    currency: 'USD',
    amount_minor: '1000',
    request_key: '24334364-9ccd-4afe-80eb-834adcf4cf33',
  } as const
  const channel = {
    id: '1',
    provider: 'stripe',
    environment: 'test',
    currency: 'USD',
  } as const
  const quote = {
    ...request,
    order_id: 'top_53',
    account_id: '39',
    provider: channel.provider,
    environment: channel.environment,
    credit_units: '5000000',
    credit_unit: 'credit_500k_usd',
  } as const
  saveNativeCheckout(
    walletSession,
    { channel, request, quote, submitted: true },
    dom.sessionStorage
  )
  const container = document.body.appendChild(document.createElement('div'))
  const root = createRoot(container)
  let navigations = 0
  container.addEventListener('click', (event) => {
    if ((event.target as HTMLElement).closest('a')) {
      event.preventDefault()
      navigations++
    }
  })
  const statusReads: string[] = []
  let resumeRefresh!: () => void
  const refreshInterrupted = new Promise<void>((resolve) => {
    resumeRefresh = resolve
  })
  let writes = 0
  globalThis.setTimeout = ((
    callback: Parameters<typeof setTimeout>[0],
    delay?: number,
    ...args: unknown[]
  ) =>
    originalTimer(
      callback,
      delay === 3000 ? 0 : delay,
      ...args
    )) as typeof setTimeout
  api.get = (async (url: string, options: { signal?: AbortSignal }) => {
    if (url.endsWith('/channels')) {
      return {
        data: { success: true, data: { available: true, channels: [channel] } },
      }
    }
    assert.equal(options.signal?.aborted, false)
    statusReads.push(url)
    const awaitingPayment = statusReads.length === 1
    if (statusReads.length === 2) {
      await refreshInterrupted
      throw Object.assign(new AxiosError('temporary gateway error'), {
        response: { status: 503 },
      })
    }
    return {
      data: {
        success: true,
        data: {
          ...quote,
          state: awaitingPayment ? 'awaiting_payment' : 'credited',
          checkout_url: 'https://pay.example/recovered',
          provider_paid: !awaitingPayment,
          core_credit_confirmed: !awaitingPayment,
        },
      },
    }
  }) as typeof api.get
  api.post = (async () => {
    writes++
    throw new Error('Unexpected payment write')
  }) as typeof api.post
  try {
    await act(async () => {
      root.render(
        <I18nextProvider i18n={i18n}>
          <NativeWalletCheckout />
        </I18nextProvider>
      )
    })
    await act(async () => {
      const deadline = Date.now() + 1000
      while (statusReads.length < 2 && Date.now() < deadline) await flush()
    })
    await click('Close')
    assert.match(container.textContent ?? '', /Details/)
    assert.doesNotMatch(container.textContent ?? '', /Refresh payment status/)
    await act(async () => {
      hidden = true
      document.dispatchEvent(new Event('visibilitychange'))
      resumeRefresh()
      await flush()
    })
    assert.equal(statusReads.length, 2, 'hidden checkout stops polling')
    await act(async () => {
      hidden = false
      document.dispatchEvent(new Event('visibilitychange'))
      const deadline = Date.now() + 1000
      while (statusReads.length < 3 && Date.now() < deadline) await flush()
    })
    assert.match(container.textContent ?? '', /Funds have reached your wallet/)
    assert.match(container.textContent ?? '', /Start another payment/)
    assert.deepEqual(
      statusReads,
      Array(3).fill('/api/user/payments/checkout/top_53')
    )
    assert.equal(
      readNativeCheckout(walletSession, dom.sessionStorage)?.request
        .request_key,
      request.request_key
    )
    await click('Start another payment')
    assert.match(container.textContent ?? '', /Review payment/)
    assert.equal(readNativeCheckout(walletSession, dom.sessionStorage), null)
    assert.equal(navigations, 0, 'restored orders never auto-navigate')
    assert.equal(writes, 0)
  } finally {
    resumeRefresh()
    await act(async () => root.unmount())
    container.remove()
    api.get = oldGet
    api.post = oldPost
    globalThis.setTimeout = originalTimer
    if (originalVisibility) {
      Object.defineProperty(document, 'hidden', originalVisibility)
    } else {
      Reflect.deleteProperty(document, 'hidden')
    }
  }
})

test('native checkout reviews exact Core quote, preserves a lost checkout and resumes by read', async () => {
  const oldGet = api.get,
    oldPost = api.post
  dom.sessionStorage.clear()
  login(19, 'checkout-session')
  const container = document.body.appendChild(document.createElement('div'))
  let root = createRoot(container)
  const writes: Array<{ url: string; data: Record<string, string> }> = []
  let coreCredit = false
  let refundState: 'refund_pending' | 'partially_refunded' | 'refunded' | null =
    null
  let accountRefreshes = 0
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
              state:
                refundState ?? (coreCredit ? 'credited' : 'credit_pending'),
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
          <NativeWalletCheckout onUpdated={() => accountRefreshes++} />
        </I18nextProvider>
      )
      await flush()
    })
  }
  try {
    await render()
    assert.ok(document.body.textContent?.includes('Test payment'))
    assert.equal(writes.length, 0)
    await click('25')
    assert.equal(
      (container.querySelector('#native-payment-amount') as HTMLInputElement)
        .value,
      '25.00'
    )
    assert.equal(writes.length, 0, 'selecting a preset must not quote or pay')
    await click('10')
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
    await click('Close')
    assert.equal(
      document.body.textContent?.includes('Refresh payment status'),
      false
    )
    assert.equal(
      readNativeCheckout('[19,"checkout-session"]', dom.sessionStorage)
        ?.submitted,
      true
    )
    await click('Details')
    assert.ok(document.body.textContent?.includes('Refresh payment status'))
    assert.equal(
      writes.length,
      2,
      'closing and reopening must not mutate the order'
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
    assert.equal(accountRefreshes, 0, 'provider payment is not wallet credit')
    coreCredit = true
    await click('Refresh payment status')
    assert.ok(
      document.body.textContent?.includes('Funds have reached your wallet.')
    )
    assert.ok(document.body.textContent?.includes('Start another payment'))
    assert.equal(accountRefreshes, 1, 'confirmed credit refreshes the balance')
    await click('Refresh payment status')
    assert.equal(accountRefreshes, 1, 'repeated status does not refresh again')
    refundState = 'refund_pending'
    await click('Refresh payment status')
    assert.ok(
      container.textContent?.includes(
        'A refund is being processed for this order.'
      )
    )
    assert.equal(
      container.textContent?.includes('Start another payment'),
      false
    )
    assert.equal(accountRefreshes, 1)
    refundState = 'partially_refunded'
    await click('Refresh payment status')
    assert.ok(
      container.textContent?.includes('Part of this payment has been refunded.')
    )
    assert.ok(container.textContent?.includes('Start another payment'))
    assert.equal(
      accountRefreshes,
      2,
      'committed partial refund refreshes the balance'
    )
    await click('Refresh payment status')
    assert.equal(
      accountRefreshes,
      3,
      'a later partial refund can keep the same state'
    )
    await render()
    assert.equal(
      accountRefreshes,
      3,
      'rendering the same response is not a new read'
    )
    refundState = 'refund_pending'
    await click('Refresh payment status')
    assert.equal(accountRefreshes, 3)
    refundState = 'partially_refunded'
    await click('Refresh payment status')
    assert.equal(
      accountRefreshes,
      4,
      'completion after another pending refund refreshes the balance'
    )
    refundState = 'refunded'
    await click('Refresh payment status')
    assert.ok(
      document.body.textContent?.includes('This payment has been refunded.')
    )
    assert.equal(accountRefreshes, 5, 'completed refund refreshes the balance')
    await click('Refresh payment status')
    assert.equal(accountRefreshes, 5)
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
            min_credit_units: '9223372036854775807',
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
    assert.equal(container.querySelector('#native-payment-minimum'), null)
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

for (const method of ['GET', 'POST'] as const) {
  test(`confirmed ${method} checkout navigates once in the current tab and keeps manual recovery`, async () => {
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
      ['requestSubmit', 'provider-data'],
    ]
    const checkoutURL = `${endpoint}?${new URLSearchParams(fields)}`
    let ready = method === 'GET'
    let navigations = 0
    container.addEventListener('click', (event) => {
      if ((event.target as HTMLElement).closest('a')) {
        event.preventDefault()
        navigations++
      }
    })
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
      state: ready ? 'awaiting_payment' : 'checkout_pending',
      currency: 'LDC',
      amount_minor: '1000',
      credit_units: '5000000',
      credit_unit: 'credit_500k_usd',
      checkout_url: ready ? checkoutURL : undefined,
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
      assert.equal(navigations + submissions, 0)
      await click('Confirm payment')
      if (!ready) {
        assert.equal(navigations + submissions, 0)
        ready = true
        await click('Refresh payment status')
      }
      assert.equal(navigations + submissions, 1)
      await click('Refresh payment status')
      assert.equal(navigations + submissions, 1)
      if (method === 'GET') {
        const link = container.querySelector('a')
        assert.ok(link)
        assert.equal(link.href, checkoutURL)
        assert.equal(link.target, '_self')
        assert.equal(container.querySelector('form'), null)
        await act(async () => link.click())
        assert.equal(navigations, 2)
      } else {
        const form = container.querySelector<HTMLFormElement>('form')
        assert.ok(form)
        assert.equal(container.querySelector('a'), null)
        assert.equal(form.method, 'post')
        assert.equal(form.action, endpoint)
        assert.equal(form.target, '_self')
        assert.equal(form.getAttribute('rel'), 'noopener noreferrer')
        assert.equal(form.enctype, 'application/x-www-form-urlencoded')
        assert.deepEqual(
          Array.from(
            form.querySelectorAll<HTMLInputElement>('input[type="hidden"]')
          ).map((input) => [input.name, input.value]),
          fields
        )
        await click('Open payment page')
        assert.equal(submissions, 2)
      }
      assert.equal(writes, 2)
    } finally {
      await act(async () => root.unmount())
      container.remove()
      api.get = oldGet
      api.post = oldPost
    }
  })
}

test('Coinbase switches USDC charge and USD credit presets while both quotes use USD', async () => {
  const oldGet = api.get,
    oldPost = api.post
  dom.sessionStorage.clear()
  login(23, 'coinbase-session')
  const container = document.body.appendChild(document.createElement('div'))
  const root = createRoot(container)
  const destinations: string[] = []
  container.addEventListener('click', (event) => {
    const anchor = (event.target as HTMLElement).closest('a')
    if (anchor) {
      event.preventDefault()
      destinations.push(anchor.href)
    }
  })
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
            preset_amounts_minor: ['300', '1000'],
          },
          {
            id: '9',
            provider: 'coinbase',
            environment: 'test',
            currency: 'USD',
            credit_quantity: true,
            preset_amounts_minor: ['200', '850'],
          },
          {
            id: '10',
            provider: 'stripe',
            environment: 'test',
            currency: 'USD',
            preset_amounts_minor: ['200', '850'],
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
              amount_minor: data.requested_credit_units
                ? '850'
                : data.amount_minor,
              credit_units: data.requested_credit_units ?? order.credit_units,
              credit_unit: order.credit_unit,
            }
          : order,
      },
    }
  }) as typeof api.post
  const chooseChannel = async (id: string) => {
    const select = container.querySelector<HTMLSelectElement>(
      '#native-payment-channel'
    )
    assert.ok(select)
    await act(async () => {
      select.value = id
      select.dispatchEvent(new Event('change', { bubbles: true }))
    })
  }
  const choices = () =>
    Array.from(
      container.querySelectorAll('[role="group"] button'),
      (button) => button.textContent
    )
  const amount = () =>
    container.querySelector<HTMLInputElement>('#native-payment-amount')?.value
  const label = () =>
    container.querySelector('label[for="native-payment-amount"]')?.textContent
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
    assert.equal(label(), 'Amount · USDC')
    assert.deepEqual(choices(), ['3', '10'])
    await click('10')
    await chooseChannel('10')
    assert.equal(label(), 'Amount · USD')
    assert.equal(amount(), '2.00')
    await click('8.5')
    await chooseChannel('8')
    assert.equal(amount(), '3.00')
    await click('10')
    await chooseChannel('9')
    assert.equal(label(), 'Wallet credit · USD')
    assert.deepEqual(choices(), ['2', '8.5'])
    assert.equal(amount(), '2.00')
    await click('8.5')
    assert.equal(requests.length, 0)
    await click('Review payment')
    assert.equal(requests.length, 1)
    assert.equal(requests[0].data.currency, 'USD')
    assert.equal(requests[0].data.amount_minor, '0')
    assert.equal(requests[0].data.requested_credit_units, '4250000')
    await click('Edit payment amount')
    await chooseChannel('8')
    assert.equal(label(), 'Amount · USDC')
    assert.equal(amount(), '3.00')
    await click('10')
    await click('Review payment')
    assert.equal(requests.length, 2)
    assert.equal(requests[1].data.currency, 'USD')
    assert.equal(requests[1].data.amount_minor, '1000')
    assert.equal(requests[1].data.requested_credit_units, undefined)
    assert.ok(container.textContent?.includes('USDC 10.00'))
    assert.ok(container.textContent?.includes('5,000,000 Credits'))
    assert.equal(container.querySelector('a'), null)
    await click('Confirm payment')
    assert.equal(requests.length, 3)
    assert.equal(requests[2].data.request_key, requests[1].data.request_key)
    assert.equal(requests[2].data.expected_credit_units, '5000000')
    assert.deepEqual(destinations, [order.checkout_url])
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

test('discount quote shows Core charge, permits editing a rejected code and confirms the frozen request', async () => {
  const { z } = await import('zod')
  const { AxiosError } = await import('axios')
  const oldAdapter = api.defaults.adapter
  dom.sessionStorage.clear()
  login(9901, 'discount-native')
  const container = document.body.appendChild(document.createElement('div'))
  const root = createRoot(container)
  const requests: Record<string, string>[] = []
  api.defaults.adapter = async (config) => {
    const response = (data: unknown, status = 200) => ({
      data,
      status,
      statusText: status === 200 ? 'OK' : 'Bad Request',
      headers: {},
      config,
    })
    if (config.url === '/api/user/payments/channels') {
      return response({
        success: true,
        data: {
          available: true,
          channels: [
            {
              id: '1',
              provider: 'stripe',
              environment: 'test',
              currency: 'USD',
              credit_quantity: true,
            },
          ],
        },
      })
    }
    assert.equal(typeof config.data, 'string')
    const body = z.record(z.string(), z.string()).parse(JSON.parse(config.data))
    requests.push(body)
    if (body.discount_code === 'BAD') {
      throw new AxiosError(
        'Rejected quote',
        'ERR_BAD_REQUEST',
        config,
        undefined,
        response({ success: false, error: 'PAYMENT_INVALID' }, 400)
      )
    }
    if (config.url === '/api/user/payments/quote') {
      return response({
        success: true,
        data: {
          ...body,
          amount_minor: '900',
          order_id: 'top_9901',
          account_id: '9901',
          provider: 'stripe',
          environment: 'test',
          credit_units: '5000000',
          credit_unit: 'credit_500k_usd',
        },
      })
    }
    assert.equal(config.url, '/api/user/payments/checkout')
    return response({
      success: true,
      data: {
        order_id: 'top_9901',
        state: 'awaiting_payment',
        currency: 'USD',
        amount_minor: '900',
        credit_units: '5000000',
        credit_unit: 'credit_500k_usd',
        provider_paid: false,
        core_credit_confirmed: false,
      },
    })
  }
  const enterCode = async (value: string) => {
    const input = document.querySelector('#native-payment-discount')
    assert.ok(input instanceof HTMLInputElement)
    const setter = Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      'value'
    )?.set
    assert.ok(setter)
    await act(async () => {
      setter.call(input, value)
      input.dispatchEvent(new Event('input', { bubbles: true }))
      await flush()
    })
  }
  try {
    await act(async () => {
      root.render(
        <I18nextProvider i18n={i18n}>
          <NativeWalletCheckout />
        </I18nextProvider>
      )
      await flush()
    })
    await enterCode('bad')
    await click('Review payment')
    assert.equal(requests[0].discount_code, 'BAD')
    await click('Edit payment amount')
    await enterCode(' save10 ')
    await click('Review payment')
    assert.equal(requests[1].discount_code, 'SAVE10')
    assert.equal(requests[1].amount_minor, '0')
    assert.notEqual(requests[0].request_key, requests[1].request_key)
    assert.match(container.querySelector('dd')?.textContent ?? '', /USD 9\.00/)
    assert.ok(container.textContent?.includes('SAVE10'))
    await click('Confirm payment')
    assert.deepEqual(requests[2], {
      ...requests[1],
      order_id: 'top_9901',
      expected_credit_units: '5000000',
      expected_amount_minor: '900',
    })
  } finally {
    await act(async () => root.unmount())
    container.remove()
    api.defaults.adapter = oldAdapter
    dom.sessionStorage.clear()
  }
})

test('a shared discount link only prefills one valid code and never starts a payment', async () => {
  const adapter = api.defaults.adapter
  const previousURL = dom.location.href
  dom.sessionStorage.clear()
  login(9902, 'discount-link-prefill')
  const container = document.body.appendChild(document.createElement('div'))
  const root = createRoot(container)
  let writes = 0
  api.defaults.adapter = async (config) => {
    if (config.method !== 'get') writes++
    return {
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: {
        success: true,
        data: {
          available: true,
          channels: [
            {
              id: '7',
              provider: 'waffo_pancake',
              environment: 'test',
              currency: 'CNY',
              credit_quantity: true,
            },
            {
              id: '8',
              provider: 'waffo_pancake',
              environment: 'test',
              currency: 'USD',
              credit_quantity: true,
            },
          ],
        },
      },
    }
  }
  try {
    for (const [search, expected] of [
      ['?discount_code=save20', 'SAVE20'],
      ['?discount_code=SAVE20&discount_code=OTHER20', ''],
      ['?discount_code=%3Cscript%3E', ''],
    ]) {
      dom.location.href = `https://wallet.example/wallet${search}`
      await act(async () => {
        root.render(
          <I18nextProvider i18n={i18n}>
            <NativeWalletCheckout key={search} />
          </I18nextProvider>
        )
        await flush()
      })
      assert.match(
        container.querySelector('label[for="native-payment-amount"]')
          ?.textContent ?? '',
        /Wallet credit.*CNY/
      )
      const channel = container.querySelector<HTMLSelectElement>(
        '#native-payment-channel'
      )
      assert.ok(channel)
      await act(async () => {
        channel.value = '8'
        channel.dispatchEvent(new Event('change', { bubbles: true }))
      })
      assert.match(
        container.querySelector('label[for="native-payment-amount"]')
          ?.textContent ?? '',
        /Wallet credit.*USD/
      )
      const input = container.querySelector('#native-payment-discount')
      assert.ok(input instanceof HTMLInputElement)
      assert.equal(input.value, expected)
      assert.equal(writes, 0)
      assert.equal(
        readNativeCheckout(
          '[9902,"discount-link-prefill"]',
          dom.sessionStorage
        ),
        null
      )
    }
  } finally {
    await act(async () => root.unmount())
    container.remove()
    dom.location.href = previousURL
    api.defaults.adapter = adapter
    dom.sessionStorage.clear()
  }
})

test('wallet presets follow the entered currency, minimum and explicit empty list', async () => {
  const { config } = useSystemConfigStore.getState()
  useSystemConfigStore.getState().setConfig({
    currency: {
      ...config.currency,
      currencyUnit: 'credit',
      creditsPerUsd: 500000,
      cnyPerUsd: 7,
      cnyPerUsdExact: '7',
    },
  })
  const oldGet = api.get,
    oldPost = api.post
  dom.sessionStorage.clear()
  login(9912, 'currency-presets')
  const container = document.body.appendChild(document.createElement('div'))
  const root = createRoot(container)
  const channels = [
    { id: '1', currency: 'USD', preset_amounts_minor: ['250', '1000'] },
    {
      id: '2',
      currency: 'CNY',
      credit_quantity: true,
      min_credit_units: '500000',
      preset_amounts_minor: ['500', '700', '3500'],
    },
    {
      id: '3',
      currency: 'EUR',
      min_amount_minor: '400',
      preset_amounts_minor: [],
    },
  ].map((channel) => ({
    ...channel,
    provider: 'waffo_pancake',
    environment: 'test',
  }))
  api.get = (async () => ({
    data: { success: true, data: { available: true, channels } },
  })) as typeof api.get
  const writes: Record<string, string>[] = []
  api.post = (async (url: string, data: Record<string, string>) => {
    assert.equal(url, '/api/user/payments/quote')
    writes.push(data)
    return {
      data: {
        success: true,
        data: {
          ...data,
          order_id: 'top_9912',
          account_id: '9912',
          provider: 'waffo_pancake',
          environment: 'test',
          credit_unit: 'credit_500k_usd',
          credit_units: '100000',
        },
      },
    }
  }) as typeof api.post
  const select = async (value: string) => {
    const select = container.querySelector<HTMLSelectElement>(
      '#native-payment-channel'
    )
    assert.ok(select)
    await act(async () => {
      select.value = value
      select.dispatchEvent(new Event('change', { bubbles: true }))
    })
  }
  const choices = () =>
    Array.from(
      container.querySelectorAll('[role="group"] button'),
      (button) => button.textContent
    )
  const input = () =>
    container.querySelector<HTMLInputElement>('#native-payment-amount')?.value
  try {
    await act(async () => {
      root.render(
        <I18nextProvider i18n={i18n}>
          <NativeWalletCheckout />
        </I18nextProvider>
      )
      await flush()
    })
    assert.deepEqual(choices(), ['2.5', '10'])
    assert.equal(input(), '2.50')
    await click('10')
    await select('2')
    assert.deepEqual(choices(), ['7', '35'])
    assert.equal(input(), '7.00')
    await enterAmount('14.50')
    assert.equal(input(), '14.50')
    await act(async () => {
      useSystemConfigStore.getState().setConfig({
        currency: {
          ...useSystemConfigStore.getState().config.currency,
          cnyPerUsd: 0,
          cnyPerUsdExact: '',
        },
      })
    })
    assert.deepEqual(choices(), [])
    assert.equal(input(), '14.50')
    await click('Review payment')
    assert.equal(writes.length, 0)
    await select('3')
    assert.deepEqual(choices(), [])
    assert.equal(input(), '')
    await enterAmount('3.99')
    await click('Review payment')
    assert.equal(writes.length, 0)
    await enterAmount('4.75')
    await click('Review payment')
    assert.equal(writes.length, 1)
    assert.equal(writes[0].currency, 'EUR')
    assert.equal(writes[0].amount_minor, '475')
  } finally {
    await act(async () => root.unmount())
    container.remove()
    api.get = oldGet
    api.post = oldPost
    useSystemConfigStore.setState({ config })
    dom.sessionStorage.clear()
  }
})

test('credit minimums block a new quote exactly and explain the required selected-currency value', async () => {
  for (const [currency, min, below, exact, wanted] of [
    ['USD', '500001', '1', '1.000002', '500001'],
    [
      'USD',
      '9007199254740993',
      '18014398509.481984',
      '18014398509.481986',
      '9007199254740993',
    ],
    ['CNY', '500000', '6.99', '7.00', '500000'],
  ] as const) {
    const originalConfig = useSystemConfigStore.getState().config
    useSystemConfigStore.getState().setConfig({
      currency: {
        ...originalConfig.currency,
        currencyUnit: 'credit',
        creditsPerUsd: 500000,
        cnyPerUsd: 7,
        cnyPerUsdExact: '7',
      },
    })
    const oldGet = api.get
    const oldPost = api.post
    dom.sessionStorage.clear()
    login(9910, 'minimum-credit')
    const container = document.body.appendChild(document.createElement('div'))
    const root = createRoot(container)
    const requests: Record<string, string>[] = []
    api.get = (async () => ({
      data: {
        success: true,
        data: {
          available: true,
          channels: [
            {
              id: '7',
              provider: 'waffo_pancake',
              environment: 'test',
              currency,
              credit_quantity: true,
              min_credit_units: min,
            },
          ],
        },
      },
    })) as typeof api.get
    api.post = (async (url: string, data: Record<string, string>) => {
      assert.equal(url, '/api/user/payments/quote')
      requests.push({ ...data })
      return {
        data: {
          success: true,
          data: {
            ...data,
            amount_minor: '700',
            order_id: 'top_9910',
            account_id: '9910',
            provider: 'waffo_pancake',
            environment: 'test',
            credit_units: data.requested_credit_units,
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
      const input = container.querySelector('#native-payment-amount')
      assert.ok(input instanceof HTMLInputElement)
      const review = Array.from(container.querySelectorAll('button')).find(
        (button) => button.textContent?.trim() === 'Review payment'
      )
      assert.ok(review)
      const hint = container.querySelector('#native-payment-minimum')
      assert.equal(
        hint?.textContent,
        currency === 'USD'
          ? 'Minimum wallet credit is USD ' + exact + '.'
          : 'Minimum CNY 7'
      )
      assert.equal(input.getAttribute('aria-describedby'), hint?.id)
      await enterAmount(below)
      assert.equal(input.getAttribute('aria-invalid'), 'true')
      assert.equal(hint?.getAttribute('role'), 'alert')
      assert.equal(review.disabled, true)
      const form = container.querySelector('form')
      assert.ok(form)
      await act(async () => {
        form.dispatchEvent(
          new Event('submit', { bubbles: true, cancelable: true })
        )
        await flush()
      })
      assert.equal(requests.length, 0)
      assert.equal(
        readNativeCheckout('[9910,"minimum-credit"]', dom.sessionStorage),
        null
      )
      await enterAmount(exact)
      assert.equal(input.getAttribute('aria-invalid'), null)
      assert.equal(review.disabled, false)
      await click('Review payment')
      assert.equal(requests.length, 1)
      assert.equal(requests[0].requested_credit_units, wanted)
      assert.equal(requests[0].currency, currency)
      assert.equal(requests[0].amount_minor, '0')
    } finally {
      await act(async () => root.unmount())
      container.remove()
      useSystemConfigStore.setState({ config: originalConfig })
      api.get = oldGet
      api.post = oldPost
      dom.sessionStorage.clear()
    }
  }
})

test('minimum increases preserve explicit recovery and retries of the original order', async () => {
  const oldGet = api.get
  const oldPost = api.post
  dom.sessionStorage.clear()
  login(9911, 'minimum-recovery')
  const container = document.body.appendChild(document.createElement('div'))
  let root = createRoot(container)
  let minimum = '25000000'
  let loseQuote = true
  let loseCheckout = true
  const writes: Array<{ url: string; data: Record<string, string> }> = []
  api.get = (async (url: string) => {
    assert.equal(url, '/api/user/payments/channels')
    return {
      data: {
        success: true,
        data: {
          available: true,
          channels: [
            {
              id: '1',
              provider: 'stripe',
              environment: 'test',
              currency: 'USD',
              credit_quantity: true,
              min_credit_units: minimum,
            },
          ],
        },
      },
    }
  }) as typeof api.get
  api.post = (async (url: string, data: Record<string, string>) => {
    writes.push({ url, data: { ...data } })
    if (url === '/api/user/payments/quote') {
      if (loseQuote) {
        loseQuote = false
        throw new Error('lost original quote response')
      }
      return {
        data: {
          success: true,
          data: {
            ...data,
            amount_minor: '5000',
            order_id: 'top_9911',
            account_id: '9911',
            provider: 'stripe',
            environment: 'test',
            credit_units: '25000000',
            credit_unit: 'credit_500k_usd',
          },
        },
      }
    }
    assert.equal(url, '/api/user/payments/checkout')
    if (loseCheckout) {
      loseCheckout = false
      throw new Error('lost original checkout response')
    }
    return {
      data: {
        success: true,
        data: {
          order_id: 'top_9911',
          state: 'awaiting_payment',
          currency: 'USD',
          amount_minor: '5000',
          credit_units: '25000000',
          credit_unit: 'credit_500k_usd',
          provider_paid: false,
          core_credit_confirmed: false,
        },
      },
    }
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
  const remount = async () => {
    await act(async () => root.unmount())
    root = createRoot(container)
    await render()
  }
  try {
    await render()
    await enterAmount('50')
    await click('Review payment')
    assert.equal(writes.length, 1)
    const original = writes[0].data
    assert.equal(original.requested_credit_units, '25000000')
    assert.equal(
      readNativeCheckout('[9911,"minimum-recovery"]', dom.sessionStorage)
        ?.quote,
      undefined
    )

    minimum = '50000000'
    await remount()
    assert.equal(writes.length, 1)
    assert.equal(container.querySelector('#native-payment-amount'), null)
    await click('Recover the same quote')
    assert.equal(writes.length, 2)
    assert.deepEqual(writes[1].data, original)

    minimum = '75000000'
    await remount()
    assert.equal(writes.length, 2)
    await click('Confirm payment')
    assert.equal(writes.length, 3)
    assert.deepEqual(writes[2].data, {
      ...original,
      order_id: 'top_9911',
      expected_credit_units: '25000000',
      expected_amount_minor: '5000',
    })

    minimum = '100000000'
    await remount()
    assert.equal(writes.length, 3)
    await click('Retry the same order')
    assert.equal(writes.length, 4)
    assert.deepEqual(writes[3], writes[2])
    assert.equal(
      readNativeCheckout('[9911,"minimum-recovery"]', dom.sessionStorage)
        ?.request.request_key,
      original.request_key
    )
  } finally {
    await act(async () => root.unmount())
    container.remove()
    api.get = oldGet
    api.post = oldPost
    dom.sessionStorage.clear()
  }
})
