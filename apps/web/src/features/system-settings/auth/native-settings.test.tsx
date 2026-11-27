/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
// @ts-expect-error Bun's test module is available only in the test runtime.
import { mock as moduleMock } from 'bun:test'
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { Window } from 'happy-dom'
import type { ReactNode } from 'react'

const dom = new Window({
  url: 'https://console.example.test/system-settings/auth/humangrant',
})
// Use Motion's JS fallback because Happy DOM rejects canceled WAAPI promises.
Reflect.deleteProperty(dom.Element.prototype, 'animate')
for (const name of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLInputElement',
  'HTMLButtonElement',
  'Node',
  'Element',
  'Event',
  'MouseEvent',
  'MutationObserver',
  'ResizeObserver',
  'DocumentFragment',
  'ShadowRoot',
  'getComputedStyle',
  'matchMedia',
  'requestAnimationFrame',
  'cancelAnimationFrame',
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

let section = 'humangrant'
let federationEnabled = true
const router = await import('@tanstack/react-router')
moduleMock.module('@tanstack/react-router', () => ({
  ...router,
  useParams: () => ({ section }),
}))
moduleMock.module('../components/settings-page', () => ({
  SettingsPage: () => {
    throw new Error(
      'Native identity settings must not depend on the legacy options editor'
    )
  },
  SettingsPageFrame: ({ children }: { children: ReactNode }) => children,
}))
moduleMock.module('@/hooks/use-status', () => ({
  useStatus: () => ({
    capabilitiesReady: true,
    status: {
      server_address: 'https://console.example.test',
      capability_scope: 'configured_components_only',
      ...(federationEnabled && { native_federated_auth: true }),
    },
  }),
}))

const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider, notifyManager } =
  await import('@tanstack/react-query')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { AuthSettings } = await import('./index')
notifyManager.setScheduler(queueMicrotask)
const i18n = createInstance()
await i18n.use(initReactI18next).init({ lng: 'en', resources: {} })
const priorAuth = useAuthStore.getState().auth
const priorAdapter = api.defaults.adapter
const priorFetch = globalThis.fetch
let dispose: (() => Promise<void>) | undefined

afterEach(async () => {
  await dispose?.()
  dispose = undefined
  federationEnabled = true
  api.defaults.adapter = priorAdapter
  globalThis.fetch = priorFetch
  useAuthStore.setState({ auth: priorAuth })
  document.body.replaceChildren()
})
after(() => dom.close())

async function mount() {
  useAuthStore.getState().auth.setBundle({
    access_token: 'synthetic-settings-session',
    token_type: 'Bearer',
    access_expires_at: 2_000_000_000,
    user: { id: 501, role: 100, username: 'synthetic-settings-admin' },
    session: {
      sid: 'synthetic-settings-sid',
      current: true,
      login_method: 'password',
      ip: '127.0.0.1',
      user_agent: 'synthetic',
      created_at: 1,
      last_active_at: 1,
      expires_at: 2_000_000_000,
    },
  })
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  await act(async () =>
    root.render(
      <QueryClientProvider client={client}>
        <I18nextProvider i18n={i18n}>
          <AuthSettings />
        </I18nextProvider>
      </QueryClientProvider>
    )
  )
  dispose = async () => {
    await act(async () => root.unmount())
    client.clear()
  }
  return container
}

async function until(check: () => boolean) {
  for (let attempt = 0; attempt < 50 && !check(); attempt++) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 10))
    })
  }
  assert.ok(check(), 'Native settings did not reach the expected state')
}

test('HumanGrant route loads its native settings without the generic options gate', async () => {
  section = 'humangrant'
  const requests: string[] = []
  globalThis.fetch = (async (input) => {
    requests.push(String(input))
    assert.equal(input, '/api/user/auth/humangrant/settings')
    return Response.json({ enabled: true, csrf: 'synthetic-settings-proof' })
  }) as typeof fetch
  api.defaults.adapter = async () =>
    assert.fail('HumanGrant does not use legacy options')
  const container = await mount()
  await until(
    () =>
      container.querySelector<HTMLInputElement>('input[type=checkbox]')
        ?.checked === true
  )
  assert.deepEqual(requests, ['/api/user/auth/humangrant/settings'])
})

test('custom OAuth route reads and saves a native provider with its exact revision', async () => {
  section = 'custom-oauth'
  // On a mixed deployment, the protected provider endpoint owns this decision.
  federationEnabled = false
  const requests: string[] = []
  const writes: Record<string, unknown>[] = []
  const provider = {
    id: '9007199254740993',
    revision: '9007199254740995',
    name: 'Native company login',
    slug: 'company',
    icon: '',
    enabled: true,
    client_id: 'synthetic-client',
    client_secret: '',
    secret_configured: true,
    authorization_endpoint: 'https://identity.example.test/authorize',
    token_endpoint: 'https://identity.example.test/token',
    user_info_endpoint: 'https://identity.example.test/userinfo',
    scopes: 'profile',
    user_id_field: 'sub',
    username_field: 'name',
    display_name_field: 'name',
    email_field: 'email',
    well_known: '',
    auth_style: 2,
    access_policy: '',
    access_denied_message: '',
  }
  api.defaults.adapter = async (config) => {
    requests.push(config.url ?? '')
    if (config.method === 'put') {
      assert.equal(config.url, `/api/custom-oauth-provider/${provider.id}`)
      writes.push(JSON.parse(config.data))
    } else {
      assert.equal(config.url, '/api/custom-oauth-provider/')
    }
    return {
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: {
        success: true,
        data: config.method === 'put' ? provider : [provider],
      },
    }
  }
  const container = await mount()
  await until(
    () => container.textContent?.includes('Native company login') === true
  )
  assert.ok(
    container.textContent?.includes('https://console.example.test/oauth/{slug}')
  )
  assert.deepEqual(requests, ['/api/custom-oauth-provider/'])
  const edit = container.querySelector<HTMLButtonElement>(
    'button[aria-label="Edit"]'
  )
  assert.ok(edit)
  await act(async () => edit.click())
  await until(
    () => document.querySelector('#custom-oauth-provider-form') !== null
  )
  const form = document.querySelector<HTMLFormElement>(
    '#custom-oauth-provider-form'
  )
  assert.ok(form)
  await act(async () => {
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
  })
  await until(() => writes.length === 1)
  assert.equal(writes[0].expected_revision, provider.revision)
  assert.match(String(writes[0].operation_key), /^[a-z\d-]{36}$/i)
  assert.equal(writes[0].client_secret, '')
})

test('failed provider reads show recovery instead of an empty editable provider table', async () => {
  section = 'custom-oauth'
  api.defaults.adapter = async () => {
    throw new Error('settings unavailable')
  }
  const container = await mount()
  await until(
    () => container.textContent?.includes('Failed to load settings') === true
  )
  assert.ok(
    !container.textContent?.includes(
      'No custom OAuth providers configured yet.'
    )
  )
  assert.ok(!container.textContent?.includes('Add Provider'))
  assert.ok(container.textContent?.includes('Retry'))
})

for (const enabled of [false, true]) {
  test(`a forbidden provider read with native federation ${enabled ? 'enabled' : 'disabled'} explains the failure inline`, async () => {
    section = 'custom-oauth'
    federationEnabled = enabled
    api.defaults.adapter = async (config) => {
      assert.equal(config.url, '/api/custom-oauth-provider/')
      assert.equal(config.skipErrorHandler, true)
      throw Object.assign(new Error('This operation is not allowed'), {
        isAxiosError: true,
        config,
        response: { status: 403, data: { code: 'AUTH_FORBIDDEN' } },
      })
    }
    const container = await mount()
    await until(() => container.textContent?.includes('Retry') === true)
    assert.equal(container.textContent?.includes('Access Forbidden'), enabled)
    assert.equal(
      container.textContent?.includes('configure its keyring'),
      !enabled
    )
    assert.ok(!container.textContent?.includes('Add Provider'))
    assert.ok(!container.textContent?.includes('Failed to load settings'))
  })
}
