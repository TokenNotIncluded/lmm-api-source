/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, beforeEach, test } from 'node:test'

import { Window } from 'happy-dom'

const dom = new Window({ url: 'https://wallet.example/wallet' })
dom.document.write('<!doctype html><html><head></head><body></body></html>')
for (const name of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLInputElement',
  'HTMLButtonElement',
  'HTMLAnchorElement',
  'SVGElement',
  'Node',
  'Element',
  'Event',
  'MouseEvent',
  'CustomEvent',
  'MutationObserver',
  'getComputedStyle',
  'requestAnimationFrame',
  'cancelAnimationFrame',
  'sessionStorage',
] as const) {
  Object.defineProperty(globalThis, name, {
    configurable: true,
    value: dom[name],
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
const { createPancakePortal } = await import('../pancake-portal-api')
const { NativeWalletCheckout } = await import('./native-wallet-checkout')
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
const oldGet = api.get,
  oldPost = api.post
const oldAuth = useAuthStore.getState().auth
const scope = { userId: 42, sessionId: 'portal-session' }
const flush = () => new Promise((resolve) => setTimeout(resolve, 15))
const portalURL =
  'https://pancake.waffo.ai/consumer/portal/store/STO_fixture#token=synthetic_portal_token'
function login(userId = 42, sessionId = 'portal-session') {
  useAuthStore.getState().auth.setBundle({
    user: { id: userId, username: `synthetic-${userId}`, role: 1 },
    access_token: 'synthetic-session-token',
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
function catalog(available: boolean) {
  api.get = (async (url: string) => {
    assert.equal(url, '/api/user/payments/channels')
    return {
      data: {
        success: true,
        data: {
          available: true,
          pancake_portal_available: available,
          channels: [
            {
              id: '7',
              provider: 'waffo_pancake',
              environment: 'test',
              currency: 'USD',
            },
          ],
        },
      },
    }
  }) as typeof api.get
}
function linkResponse() {
  return {
    data: {
      success: true,
      message: '',
      data: {
        portal_url: portalURL,
        expires_at: new Date(Date.now() + 60_000).toISOString(),
      },
    },
  }
}
async function mount() {
  const container = document.body.appendChild(document.createElement('div'))
  const root = createRoot(container)
  await act(async () => {
    root.render(
      <I18nextProvider i18n={i18n}>
        <NativeWalletCheckout />
      </I18nextProvider>
    )
    await flush()
  })
  await act(async () => {
    await flush()
  })
  return {
    container,
    close: async () => {
      await act(async () => root.unmount())
      container.remove()
    },
  }
}
async function click(label: string) {
  const button = [...document.querySelectorAll('button')].find(
    (element) => element.textContent?.trim() === label
  )
  assert.ok(button, label)
  await act(async () => {
    button.click()
    await flush()
  })
}
beforeEach(() => {
  dom.sessionStorage.clear()
  login()
  catalog(true)
})
afterEach(() => {
  api.get = oldGet
  api.post = oldPost
  useAuthStore.setState({ auth: oldAuth })
  document.body.replaceChildren()
})
after(() => dom.close())

test('Pancake channels without an available portal do not advertise or request a link', async () => {
  catalog(false)
  let calls = 0
  api.post = (async () => {
    calls++
    throw new Error('unsupported portal must not be requested')
  }) as typeof api.post
  const view = await mount()
  try {
    assert.ok(view.container.querySelector('#native-payment-channel'))
    assert.doesNotMatch(
      view.container.textContent ?? '',
      /Pancake customer portal/
    )
    assert.equal(calls, 0)
    assert.equal(view.container.querySelector('a'), null)
  } finally {
    await view.close()
  }
})

test('portal creation has an empty body and a session-bound private link opens only through a no-referrer user action', async () => {
  let calls = 0
  const response = linkResponse()
  api.post = (async (
    url: string,
    body: unknown,
    options: Record<string, unknown>
  ) => {
    assert.equal(url, '/api/user/payments/pancake/portal')
    assert.equal(body, undefined)
    assert.deepEqual(options.authScope, scope)
    assert.equal(options.skipAuthRefresh, true)
    assert.equal(options.disableDuplicate, true)
    calls++
    return response
  }) as typeof api.post
  const view = await mount()
  const clock = Date.now
  try {
    assert.equal(calls, 0)
    assert.equal(view.container.querySelector('a'), null)
    await click('Create portal link')
    assert.equal(calls, 1)
    const link = view.container.querySelector<HTMLAnchorElement>('a')
    assert.ok(link)
    assert.equal(link.href, portalURL)
    assert.equal(link.target, '_blank')
    assert.equal(link.rel, 'noopener noreferrer')
    assert.equal(link.getAttribute('referrerpolicy'), 'no-referrer')
    assert.equal(window.location.href, 'https://wallet.example/wallet')
    assert.equal(
      dom.sessionStorage.length,
      0,
      'portal token must not enter checkout recovery storage'
    )
    // A suspended tab can resume after expiry before its timer has fired.
    Date.now = () => Date.parse(response.data.data.expires_at) + 1
    await act(async () => {
      assert.equal(
        link.dispatchEvent(
          new MouseEvent('click', { bubbles: true, cancelable: true })
        ),
        false
      )
      await flush()
    })
    assert.equal(view.container.querySelector('a'), null)
    assert.match(view.container.textContent ?? '', /portal link has expired/)
    assert.equal(
      calls,
      1,
      'expired navigation does not silently mint another token'
    )
  } finally {
    Date.now = clock
    await view.close()
  }
})

test('portal response contract rejects unrelated or expired URLs and does not display supplier errors', async () => {
  const original = linkResponse()
  for (const [portal_url, expires_at] of [
    [
      'https://other.example/consumer/portal/store/STO_fixture#token=synthetic',
      original.data.data.expires_at,
    ],
    [
      'https://user:secret@pancake.waffo.ai/consumer/portal/store/STO_fixture#token=synthetic',
      original.data.data.expires_at,
    ],
    [
      'https://pancake.waffo.ai/consumer/portal/store/STO_fixture?customer=other#token=synthetic',
      original.data.data.expires_at,
    ],
    [
      'https://pancake.waffo.ai/another/path#token=synthetic',
      original.data.data.expires_at,
    ],
    [portalURL, '2000-01-01T00:00:00Z'],
  ]) {
    api.post = (async () => ({
      data: { success: true, data: { portal_url, expires_at } },
    })) as typeof api.post
    await assert.rejects(createPancakePortal(scope))
  }
  let calls = 0
  api.post = (async () => {
    calls++
    throw new Error('synthetic_provider_secret')
  }) as typeof api.post
  const view = await mount()
  try {
    await click('Create portal link')
    assert.equal(calls, 1)
    assert.match(
      view.container.textContent ?? '',
      /Payment portal is unavailable/
    )
    assert.doesNotMatch(
      view.container.textContent ?? '',
      /synthetic_provider_secret/
    )
    assert.equal(view.container.querySelector('a'), null)
    api.post = (async () => {
      calls++
      return linkResponse()
    }) as typeof api.post
    await click('Create portal link')
    assert.equal(calls, 2)
    assert.equal(
      view.container.querySelector('a')?.getAttribute('href'),
      portalURL
    )
  } finally {
    await view.close()
  }
})

test('changing session discards an in-flight portal token and never requests one for the next user', async () => {
  let complete!: (value: ReturnType<typeof linkResponse>) => void
  const pending = new Promise<ReturnType<typeof linkResponse>>((resolve) => {
    complete = resolve
  })
  let calls = 0
  api.post = (async () => {
    calls++
    return pending
  }) as typeof api.post
  const view = await mount()
  try {
    await click('Create portal link')
    assert.equal(calls, 1)
    await act(async () => {
      login(43, 'next-portal-session')
      await flush()
    })
    await act(async () => {
      complete(linkResponse())
      await flush()
    })
    assert.equal(calls, 1)
    assert.equal(view.container.querySelector('a'), null)
    assert.doesNotMatch(view.container.innerHTML, /synthetic_portal_token/)
    assert.equal(dom.sessionStorage.length, 0)
  } finally {
    await view.close()
  }
})
