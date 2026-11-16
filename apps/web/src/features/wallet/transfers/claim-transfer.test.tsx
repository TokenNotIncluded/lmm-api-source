/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import type { AxiosResponse, InternalAxiosRequestConfig } from 'axios'
import { Window } from 'happy-dom'

const dom = new Window({ url: 'https://synthetic.example/transfer' })
dom.document.write('<!doctype html><html><body></body></html>')
Object.defineProperty(dom.document, 'compatMode', {
  configurable: true,
  value: 'CSS1Compat',
})
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLButtonElement',
  'HTMLAnchorElement',
  'SVGElement',
  'Node',
  'Element',
  'Event',
  'MouseEvent',
  'CustomEvent',
  'MutationObserver',
  'ResizeObserver',
  'getComputedStyle',
  'requestAnimationFrame',
  'cancelAnimationFrame',
  'matchMedia',
  'sessionStorage',
  'localStorage',
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
const { toast } = await import('sonner')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { ClaimTransfer } = await import('./claim-transfer')

const i18n = createInstance()
await i18n.use(initReactI18next).init({
  lng: 'en',
  resources: { en: { translation: {} } },
})
const adapter = api.defaults.adapter
const originalAuth = useAuthStore.getState().auth
const storage = dom.sessionStorage
const mounted: Array<{
  root: ReturnType<typeof createRoot>
  client: InstanceType<typeof QueryClient>
}> = []
const token = 'a'.repeat(64)
const pendingKey = 'wallet-transfer-login'
const flush = () => new Promise((resolve) => setTimeout(resolve, 25))
const pending = {
  quota: 125,
  status: 'pending',
  created_at: 1700000000,
  claimed_at: 0,
  is_sender: false,
  claimed_by_me: false,
}
const confirmed = {
  ...pending,
  status: 'claimed',
  claimed_at: 1700000100,
  claimed_by_me: true,
}

function login(userId = 52, sessionId = 'claim-session-52') {
  useAuthStore.getState().auth.setBundle({
    user: { id: userId, role: 1, username: 'synthetic' },
    access_token: 'synthetic-claim-token',
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
function reply(
  config: InternalAxiosRequestConfig,
  data: unknown
): AxiosResponse {
  return {
    config,
    data: { success: true, message: '', data },
    headers: {},
    status: 200,
    statusText: 'OK',
  }
}
function shellReply(config: InternalAxiosRequestConfig): AxiosResponse {
  assert.equal(config.method, 'get')
  switch (config.url) {
    case '/api/notice':
      return reply(config, '')
    case '/api/status':
      return reply(config, { announcements_enabled: false, announcements: [] })
    case '/api/site/navigation':
      return reply(config, { value: '{}', revision: 1 })
    case '/api/ratio-notifications':
      return reply(config, [])
    default:
      throw new Error(`Unexpected shell request: ${config.url}`)
  }
}
async function render(hash = token) {
  const node = document.body.appendChild(document.createElement('div'))
  const root = createRoot(node)
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const router = createRouter({
    routeTree: createRootRoute(),
    history: createMemoryHistory({
      initialEntries: [`/transfer${hash ? `#${hash}` : ''}`],
    }),
  })
  mounted.push({ root, client })
  await act(async () => {
    root.render(
      <RouterContextProvider router={router}>
        <QueryClientProvider client={client}>
          <I18nextProvider i18n={i18n}>
            <ClaimTransfer />
          </I18nextProvider>
        </QueryClientProvider>
      </RouterContextProvider>
    )
    await flush()
    await flush()
  })
  return { client, router }
}
function claimButton() {
  const found = [...document.querySelectorAll('main button')].find(
    (element) => element.textContent?.trim() === 'Claim transfer'
  )
  assert.ok(found instanceof HTMLButtonElement)
  assert.equal(found.disabled, false)
  return found
}
function denyRecoveryStorage() {
  const calls = { reads: 0, writes: 0, removes: 0 }
  Object.defineProperty(globalThis, 'sessionStorage', {
    configurable: true,
    value: {
      getItem(key: string) {
        if (key === pendingKey) {
          calls.reads++
          throw new Error('storage blocked')
        }
        return storage.getItem(key)
      },
      setItem(key: string, value: string) {
        if (key === pendingKey) {
          calls.writes++
          throw new Error('storage blocked')
        }
        return storage.setItem(key, value)
      },
      removeItem(key: string) {
        if (key === pendingKey) {
          calls.removes++
          throw new Error('storage blocked')
        }
        return storage.removeItem(key)
      },
    },
  })
  return calls
}
afterEach(async () => {
  for (const { root, client } of mounted.splice(0)) {
    await act(async () => root.unmount())
    client.clear()
  }
  Object.defineProperty(globalThis, 'sessionStorage', {
    configurable: true,
    value: storage,
  })
  storage.clear()
  document.body.innerHTML = ''
  api.defaults.adapter = adapter
  useAuthStore.setState({ auth: originalAuth })
})
after(() => dom.close())

test('a valid bearer fragment survives unavailable recovery storage and confirmed cleanup failure', async () => {
  login()
  const calls = denyRecoveryStorage()
  const beforeToasts = toast.getHistory().length
  const requests: string[] = []
  api.defaults.adapter = async (config) => {
    if (!config.url?.startsWith('/api/wallet-transfer/')) {
      return shellReply(config)
    }
    assert.deepEqual(config.authScope, {
      userId: 52,
      sessionId: 'claim-session-52',
    })
    assert.equal(config.skipAuthRefresh, true)
    assert.equal(config.method, 'post')
    assert.equal(JSON.parse(String(config.data)).token, token)
    requests.push(config.url)
    return reply(config, config.url.endsWith('/claim') ? confirmed : pending)
  }
  await render()
  assert.match(
    document.querySelector('main')?.textContent ?? '',
    /125 Core credits/
  )
  assert.equal(
    calls.reads,
    0,
    'a valid fragment must not read recovery storage'
  )
  await act(async () => {
    claimButton().click()
    await flush()
    await flush()
  })
  assert.match(
    document.querySelector('main')?.textContent ?? '',
    /Transfer claimed. Your wallet has been credited./
  )
  assert.equal(calls.removes, 1)
  assert.deepEqual(requests, [
    '/api/wallet-transfer/inspect',
    '/api/wallet-transfer/claim',
  ])
  assert.ok(
    toast
      .getHistory()
      .slice(beforeToasts)
      .some(
        (item) =>
          'type' in item &&
          'title' in item &&
          item.type === 'success' &&
          item.title === 'Transfer claimed. Your wallet has been credited.'
      )
  )
})

test('unavailable login recovery prevents navigation without putting the bearer in a redirect', async () => {
  useAuthStore.getState().auth.reset()
  const calls = denyRecoveryStorage()
  let transferRequests = 0
  api.defaults.adapter = async (config) => {
    if (config.url?.startsWith('/api/wallet-transfer/')) {
      transferRequests++
      throw new Error('An anonymous account cannot inspect or claim a transfer')
    }
    return shellReply(config)
  }
  const { router } = await render()
  const signIn = document.querySelector<HTMLAnchorElement>(
    'main a[href^="/sign-in"]'
  )
  assert.ok(signIn)
  assert.ok(!signIn.href.includes(token))
  const click = new MouseEvent('click', {
    bubbles: true,
    cancelable: true,
    button: 0,
  })
  await act(async () => {
    signIn.dispatchEvent(click)
    await flush()
  })
  assert.equal(click.defaultPrevented, true)
  assert.equal(router.state.location.pathname, '/transfer')
  assert.equal(router.state.location.hash, token)
  assert.match(
    document.querySelector('main')?.textContent ?? '',
    /Recovery storage is unavailable/
  )
  assert.equal(calls.writes, 1)
  assert.equal(calls.reads, 0)
  assert.equal(transferRequests, 0)
})

test('blocked recovery reads render a clear error when there is no bearer fragment', async () => {
  useAuthStore.getState().auth.reset()
  const calls = denyRecoveryStorage()
  api.defaults.adapter = async (config) => shellReply(config)
  await render('')
  assert.match(
    document.querySelector('main')?.textContent ?? '',
    /Recovery storage is unavailable/
  )
  assert.equal(document.querySelector('main a[href^="/sign-in"]'), null)
  assert.equal(calls.reads, 1)
})

test('a late claim receipt cannot update or clear recovery for a replacement account', async () => {
  login()
  storage.setItem(pendingKey, 'c'.repeat(64))
  let finish: (() => void) | undefined
  let writes = 0
  api.defaults.adapter = async (config) => {
    if (!config.url?.startsWith('/api/wallet-transfer/')) {
      return shellReply(config)
    }
    if (config.url.endsWith('/claim')) {
      writes++
      assert.deepEqual(config.authScope, {
        userId: 52,
        sessionId: 'claim-session-52',
      })
      return await new Promise<AxiosResponse>((resolve) => {
        finish = () => resolve(reply(config, confirmed))
      })
    }
    return reply(config, {
      ...pending,
      quota: config.authScope?.userId === 53 ? 777 : 125,
    })
  }
  const { client } = await render()
  await act(async () => {
    claimButton().click()
    await flush()
  })
  assert.ok(finish)
  await act(async () => {
    login(53, 'claim-session-53')
    await flush()
    await flush()
  })
  assert.match(
    document.querySelector('main')?.textContent ?? '',
    /777 Core credits/
  )
  const beforeToasts = toast.getHistory().length
  const complete = finish
  await act(async () => {
    complete()
    await flush()
    await flush()
  })
  assert.equal(writes, 1)
  assert.equal(storage.getItem(pendingKey), 'c'.repeat(64))
  assert.equal(toast.getHistory().length, beforeToasts)
  assert.doesNotMatch(
    document.querySelector('main')?.textContent ?? '',
    /wallet has been credited/
  )
  assert.deepEqual(
    client.getQueryData([
      'wallet-transfer-receipt',
      53,
      'claim-session-53',
      token,
    ]),
    {
      ...pending,
      quota: 777,
    }
  )
})

test('a malformed nonempty fragment never substitutes a saved transfer bearer', async () => {
  login()
  storage.setItem(pendingKey, token)
  const calls = denyRecoveryStorage()
  const requests: string[] = []
  api.defaults.adapter = async (config) => {
    if (config.url?.startsWith('/api/wallet-transfer/')) {
      requests.push(config.url)
      throw new Error(
        'A malformed link must not inspect or claim another transfer'
      )
    }
    return shellReply(config)
  }
  await render('invalid')
  assert.match(
    document.querySelector('main')?.textContent ?? '',
    /Invalid transfer link./
  )
  assert.equal(document.querySelector('main button'), null)
  assert.deepEqual(requests, [])
  assert.equal(calls.reads, 0)
  assert.equal(storage.getItem(pendingKey), token)
})

test('an empty fragment restores the saved bearer after sign-in without claiming automatically', async () => {
  login()
  storage.setItem(pendingKey, token)
  const requests: string[] = []
  api.defaults.adapter = async (config) => {
    if (!config.url?.startsWith('/api/wallet-transfer/')) {
      return shellReply(config)
    }
    requests.push(config.url)
    assert.equal(config.url, '/api/wallet-transfer/inspect')
    assert.equal(JSON.parse(String(config.data)).token, token)
    return reply(config, pending)
  }
  await render('')
  assert.match(
    document.querySelector('main')?.textContent ?? '',
    /125 Core credits/
  )
  assert.ok(claimButton())
  assert.deepEqual(requests, ['/api/wallet-transfer/inspect'])
  assert.equal(storage.getItem(pendingKey), token)
})
