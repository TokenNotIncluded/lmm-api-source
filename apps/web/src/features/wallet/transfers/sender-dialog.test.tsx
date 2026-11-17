/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { Window } from 'happy-dom'

const dom = new Window({
  url: 'https://synthetic.example/wallet',
  width: 390,
  height: 844,
})
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLInputElement',
  'HTMLButtonElement',
  'SVGElement',
  'Node',
  'Element',
  'Event',
  'CustomEvent',
  'MutationObserver',
  'ResizeObserver',
  'getComputedStyle',
  'requestAnimationFrame',
  'cancelAnimationFrame',
  'sessionStorage',
  'customElements',
  'CSSStyleSheet',
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
const { WalletTransferDialog } = await import('./sender-dialog')
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
const adapter = api.defaults.adapter
const originalAuth = useAuthStore.getState().auth
const mounted: ReturnType<typeof createRoot>[] = []
const flush = () => new Promise((resolve) => setTimeout(resolve, 20))
const storageKey = 'wallet-transfer-resume:52'
const balance = {
  schema_version: 2,
  currency_unit: 'CREDIT',
  credits_per_usd: 500000,
  public_credits_per_usd_exact: '500000',
  spendable_credits: 1000,
}
function login(userId = 52, sessionId = 'synthetic-transfer-session') {
  useAuthStore.getState().auth.setBundle({
    user: { id: userId, role: 1, username: 'synthetic' },
    access_token: 'synthetic-access-token',
    token_type: 'Bearer',
    access_expires_at: 9999999999,
    session: {
      sid: sessionId,
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
async function render(onUpdated?: () => void) {
  const node = document.body.appendChild(document.createElement('div'))
  const root = createRoot(node)
  mounted.push(root)
  await act(async () => {
    root.render(
      <I18nextProvider i18n={i18n}>
        <WalletTransferDialog
          open
          onOpenChange={() => undefined}
          onUpdated={onUpdated}
        />
      </I18nextProvider>
    )
    await flush()
    await flush()
  })
  return root
}
function button(label: string) {
  const found = [...document.querySelectorAll('button')].find(
    (element) => element.textContent?.trim() === label
  )
  assert.ok(found, label)
  return found
}
async function click(label: string) {
  const target = button(label)
  assert.equal(target.disabled, false, label)
  await act(async () => {
    target.click()
    await flush()
    await flush()
  })
}
async function enter(value: string) {
  const input = document.querySelector('input')
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
afterEach(async () => {
  for (const root of mounted.splice(0)) await act(async () => root.unmount())
  document.body.innerHTML = ''
  sessionStorage.clear()
  api.defaults.adapter = adapter
  useAuthStore.setState({ auth: originalAuth })
})
after(() => dom.close())

test('transfers keep exact amounts and original recovery across lost acknowledgement, confirmation and cancellation', async () => {
  login()
  let row: {
    id: number
    quota: number
    status: 'awaiting_confirmation' | 'pending' | 'cancelled'
    created_at: number
    request_key: string
    token?: string
    claim_link_available?: boolean
  } | null = null
  let writes = 0
  let updates = 0
  const reads: string[] = []
  let copied = ''
  Object.defineProperty(navigator, 'clipboard', {
    configurable: true,
    value: {
      writeText: async (text: string) => {
        copied = text
      },
    },
  })
  api.defaults.adapter = async (config) => {
    assert.deepEqual(config.authScope, {
      userId: 52,
      sessionId: 'synthetic-transfer-session',
    })
    assert.equal(
      config.skipAuthRefresh,
      config.method === 'get' ? undefined : true
    )
    let data: unknown
    if (config.method === 'get') {
      assert.ok(config.url)
      reads.push(config.url)
      if (config.url === '/api/wallet/balance') {
        data = { success: true, data: balance }
      } else if (config.url === '/api/wallet-transfer') {
        data = { success: true, data: row ? [row] : [] }
      } else {
        assert.equal(config.url, '/api/wallet-transfer/query')
        assert.ok(row)
        const cancel = !!config.params.cancel_transfer_id
        if (cancel) assert.equal(config.params.cancel_transfer_id, row.id)
        else assert.equal(config.params.request_key, row.request_key)
        data =
          row.status === 'awaiting_confirmation'
            ? {
                success: false,
                code: 'WALLET_CONFIRMATION_REQUIRED',
                data: {
                  request_key: row.request_key,
                  transfer: row,
                  confirmation_url: `https://synthetic.example/core/v1/wallet/confirm/create/${row.id}`,
                },
              }
            : { success: true, data: row }
      }
    } else {
      writes++
      const savedText = sessionStorage.getItem(storageKey)
      assert.ok(savedText, 'persist the immutable operation before any write')
      const saved = JSON.parse(savedText)
      if (config.url === '/api/wallet-transfer') {
        const body = JSON.parse(String(config.data))
        assert.equal(body.quota, 125)
        assert.equal(body.request_key, saved.requestKey)
        row = {
          id: 17,
          quota: body.quota,
          status: 'awaiting_confirmation',
          created_at: 1700000000,
          request_key: body.request_key,
        }
        throw new Error('lost create acknowledgement')
      }
      assert.equal(config.url, '/api/wallet-transfer/17/cancel')
      assert.equal(saved.action, 'cancel')
      assert.equal(saved.id, 17)
      assert.ok(row)
      data = {
        success: false,
        code: 'WALLET_CONFIRMATION_REQUIRED',
        data: {
          request_key: row.request_key,
          transfer: row,
          confirmation_url:
            'https://synthetic.example/core/v1/wallet/confirm/cancel/17',
        },
      }
    }
    return { config, data, headers: {}, status: 200, statusText: 'OK' }
  }
  const root = await render(() => {
    updates++
  })
  assert.match(document.body.textContent ?? '', /1,000 Core credits/)
  for (const invalid of ['0', '1.25', '9007199254740992', '1001']) {
    await enter(invalid)
    assert.equal(button('Create transfer link').disabled, true)
  }
  await enter('125')
  await click('Create transfer link')
  assert.equal(writes, 1)
  assert.match(document.body.textContent ?? '', /result is unknown/)
  await act(async () => root.unmount())
  mounted.splice(mounted.indexOf(root), 1)
  await render(() => {
    updates++
  })
  assert.equal(writes, 1, 'remount must never replay a financial POST')
  await click('Refresh payment status')
  const createLink = document.querySelector<HTMLAnchorElement>(
    'a[href*="/confirm/create/17"]'
  )
  assert.ok(createLink)
  assert.equal(createLink.target, '_blank')
  const savedConfirmation = sessionStorage.getItem(storageKey)
  assert.ok(savedConfirmation)
  assert.equal(JSON.parse(savedConfirmation).step.kind, 'confirmation')
  const created = row as import('./api').WalletTransfer | null
  assert.ok(created)
  created.status = 'pending'
  created.token = 'a'.repeat(64)
  created.claim_link_available = true
  await click('Refresh payment status')
  assert.equal(sessionStorage.getItem(storageKey), null)
  assert.equal(writes, 1)
  await click('Copy link')
  assert.equal(copied, `https://synthetic.example/transfer#${'a'.repeat(64)}`)
  await click('Cancel transfer')
  assert.equal(writes, 2)
  assert.ok(document.querySelector('a[href*="/confirm/cancel/17"]'))
  created.status = 'cancelled'
  created.claim_link_available = false
  delete created.token
  await click('Refresh payment status')
  assert.equal(writes, 2)
  assert.equal(sessionStorage.getItem(storageKey), null)
  assert.equal(updates, 2)
  assert.ok(reads.includes('/api/wallet/balance'))
  assert.match(document.body.textContent ?? '', /Cancelled/)
})

test('corrupt recovery storage disables new transfers and a replaced account cannot see the original operation', async () => {
  login()
  sessionStorage.setItem(storageKey, '{invalid')
  let writes = 0
  api.defaults.adapter = async (config) => {
    if (config.method !== 'get') writes++
    return {
      config,
      data: {
        success: true,
        data: config.url === '/api/wallet/balance' ? balance : [],
      },
      headers: {},
      status: 200,
      statusText: 'OK',
    }
  }
  await render()
  assert.match(
    document.body.textContent ?? '',
    /Recovery storage is unavailable/
  )
  assert.equal(button('Create transfer link').disabled, true)
  assert.equal(writes, 0)
  await act(async () => {
    login(53, 'replacement-transfer-session')
    await flush()
    await flush()
  })
  assert.doesNotMatch(
    document.body.textContent ?? '',
    /Recovery storage is unavailable/
  )
  assert.equal(sessionStorage.getItem(storageKey), '{invalid')
  assert.equal(writes, 0)
})

test('unavailable transfer authority remains unavailable instead of exposing a zero balance', async () => {
  login()
  api.defaults.adapter = async () => {
    throw new Error('wallet module not ready')
  }
  await render()
  assert.match(
    document.body.textContent ?? '',
    /Failed to load transfer history/
  )
  assert.match(document.body.textContent ?? '', /Unavailable/)
  assert.doesNotMatch(document.body.textContent ?? '', /0 Core credits/)
  assert.equal(button('Create transfer link').disabled, true)
})

test('history and balance failures stay separate and do not discard successful reads', async () => {
  login()
  let failedRead = '/api/wallet-transfer'
  api.defaults.adapter = async (config) => {
    assert.equal(config.method, 'get')
    if (config.url === failedRead) throw new Error('synthetic read failure')
    return {
      config,
      data: {
        success: true,
        data: config.url === '/api/wallet/balance' ? balance : [],
      },
      headers: {},
      status: 200,
      statusText: 'OK',
    }
  }
  await render()
  assert.match(document.body.textContent ?? '', /1,000 Core credits/)
  assert.match(
    document.body.textContent ?? '',
    /Failed to load transfer history/
  )
  assert.doesNotMatch(
    document.body.textContent ?? '',
    /Failed to load wallet balance/
  )
  assert.equal(button('Create transfer link').disabled, true)

  failedRead = '/api/wallet/balance'
  await click('Refresh')
  assert.match(document.body.textContent ?? '', /Failed to load wallet balance/)
  assert.doesNotMatch(
    document.body.textContent ?? '',
    /Failed to load transfer history/
  )
  assert.match(document.body.textContent ?? '', /No transfers yet/)
  assert.match(document.body.textContent ?? '', /Unavailable/)
  assert.equal(button('Create transfer link').disabled, true)
})

test('a historical Unicode request key survives reviewing and reopening its saved confirmation', async () => {
  login()
  const requestKey = '界界界界界a'
  assert.equal(new TextEncoder().encode(requestKey).byteLength, 16)
  const row = {
    id: 17,
    quota: 125,
    status: 'awaiting_confirmation',
    created_at: 1700000000,
    request_key: requestKey,
    confirmation_url:
      'https://synthetic.example/core/v1/wallet/confirm/create/17',
  }
  let queries = 0
  let writes = 0
  api.defaults.adapter = async (config) => {
    if (config.method !== 'get') {
      writes++
      throw new Error(
        'Reviewing a saved transfer must not create another operation'
      )
    }
    let data: unknown
    if (config.url === '/api/wallet/balance') {
      data = { success: true, data: balance }
    } else if (config.url === '/api/wallet-transfer') {
      data = { success: true, data: [row] }
    } else {
      assert.equal(config.url, '/api/wallet-transfer/query')
      assert.equal(config.params.request_key, requestKey)
      queries++
      data = {
        success: false,
        code: 'WALLET_CONFIRMATION_REQUIRED',
        data: {
          request_key: requestKey,
          transfer: row,
          confirmation_url: row.confirmation_url,
        },
      }
    }
    return { config, data, headers: {}, status: 200, statusText: 'OK' }
  }
  const first = await render()
  await click('Review')
  const saved = sessionStorage.getItem(storageKey)
  assert.ok(saved)
  assert.equal(JSON.parse(saved).requestKey, requestKey)
  await act(async () => first.unmount())
  mounted.splice(mounted.indexOf(first), 1)
  await render()
  assert.doesNotMatch(
    document.body.textContent ?? '',
    /Recovery storage is unavailable/
  )
  assert.ok(document.querySelector('a[href*="/confirm/create/17"]'))
  assert.equal(button('Refresh payment status').disabled, false)
  assert.equal(queries, 0)
  await click('Refresh payment status')
  assert.equal(queries, 1)
  assert.equal(writes, 0)
  const recovered = sessionStorage.getItem(storageKey)
  assert.ok(recovered)
  assert.equal(JSON.parse(recovered).requestKey, requestKey)
  assert.equal(JSON.parse(recovered).quota, 125)
})

test('an oversized UTF-8 request key cannot reopen as a valid saved transfer', async () => {
  login()
  const requestKey = '界'.repeat(22)
  assert.equal(requestKey.length, 22)
  assert.equal(new TextEncoder().encode(requestKey).byteLength, 66)
  sessionStorage.setItem(
    storageKey,
    JSON.stringify({
      action: 'create',
      requestKey,
      quota: 125,
      step: { kind: 'unknown' },
    })
  )
  let writes = 0
  api.defaults.adapter = async (config) => {
    if (config.method !== 'get') {
      writes++
      throw new Error('Invalid recovery cannot send a financial request')
    }
    assert.ok(
      ['/api/wallet/balance', '/api/wallet-transfer'].includes(config.url ?? '')
    )
    return {
      config,
      data: {
        success: true,
        data: config.url === '/api/wallet/balance' ? balance : [],
      },
      headers: {},
      status: 200,
      statusText: 'OK',
    }
  }
  await render()
  assert.match(
    document.body.textContent ?? '',
    /Recovery storage is unavailable/
  )
  assert.equal(button('Create transfer link').disabled, true)
  assert.equal(writes, 0)
})
