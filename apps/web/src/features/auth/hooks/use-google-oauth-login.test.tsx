/*
Copyright (C) 2026 LIghtJUNction

SPDX-License-Identifier: AGPL-3.0-or-later
*/
// @ts-expect-error Bun's test module is available only in the test runtime.
import { mock } from 'bun:test'
import assert from 'node:assert/strict'
import { after, afterEach, describe, test } from 'node:test'

import { Window } from 'happy-dom'

import { installSupportedAuthBrowser } from '@/test-support/supported-auth-browser'

import { GOOGLE_BUILT_IN_PRESET } from '../lib/google-oauth'
import type { SystemStatus } from '../types'

const dom = new Window({ url: 'https://site.example.test/sign-in' })
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'Node',
  'Element',
  'Event',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: dom[key],
  })
}
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
const calls: { path: string; payload: unknown; options: unknown }[] = []
const navigations: string[] = []
const errors: string[] = []
Object.defineProperty(dom.location, 'assign', {
  configurable: true,
  value: (url: string) => navigations.push(url),
})
let response: unknown
let forbiddenCalls = 0
const t = (key: string) => key
mock.module('react-i18next', () => ({ useTranslation: () => ({ t }) }))
mock.module('sonner', () => ({
  toast: { error: (value: string) => errors.push(value), success: () => {} },
}))
mock.module('@/lib/api', () => ({
  api: {
    post: async (path: string, payload: unknown, options: unknown) => {
      calls.push({ path, payload, options })
      return { data: response }
    },
  },
  clearAuthentication: () => {
    forbiddenCalls++
  },
  isAuthBundle: () => false,
}))
mock.module('../api', () => ({
  logout: async () => {
    forbiddenCalls++
    throw new Error('not needed for Google bridge')
  },
  startTelegramLogin: async () => '',
  telegramLogin: async () => ({ success: false }),
}))
mock.module('./use-auth-redirect', () => ({
  useAuthRedirect: () => ({
    handleLoginSuccess: async () => {
      forbiddenCalls++
    },
  }),
}))
// Normal supported browser; install after this fixture replaces its globals.
installSupportedAuthBrowser()

const { act, useLayoutEffect } = await import('react')
const { createRoot } = await import('react-dom/client')
const { useOAuthLogin } = await import('./use-oauth-login')
let root: ReturnType<typeof createRoot> | undefined
let login: () => Promise<void>
function Harness({
  status,
  acceptedLegal,
}: {
  status: SystemStatus
  acceptedLegal: boolean
}) {
  const { handleGoogleLogin } = useOAuthLogin(
    status,
    '/dashboard',
    acceptedLegal
  )
  useLayoutEffect(() => {
    login = handleGoogleLogin
  }, [handleGoogleLogin])
  return null
}
const supported = { google_oauth: true, google_client_id: 'test-client' }
function validResponse() {
  const url = new URL(GOOGLE_BUILT_IN_PRESET.authorization_endpoint)
  for (const [key, value] of Object.entries({
    client_id: 'test-client',
    response_type: 'code',
    scope: 'openid email profile',
    state: 'synthetic-state',
    nonce: 'synthetic-nonce',
    code_challenge: 'synthetic-proof',
    code_challenge_method: 'S256',
    redirect_uri: 'https://site.example.test/api/oauth/google/callback',
  })) {
    url.searchParams.set(key, value)
  }
  return { success: true, data: { authorization_url: url.toString() } }
}
async function mount(status: SystemStatus = supported, acceptedLegal = false) {
  const host = document.createElement('div')
  document.body.append(host)
  const currentRoot = createRoot(host)
  root = currentRoot
  await act(async () =>
    currentRoot.render(
      <Harness status={status} acceptedLegal={acceptedLegal} />
    )
  )
}
afterEach(async () => {
  const currentRoot = root
  if (currentRoot) await act(async () => currentRoot.unmount())
  root = undefined
  document.body.replaceChildren()
  calls.length = navigations.length = errors.length = forbiddenCalls = 0
})
after(() => dom.close())

describe('built-in Google hook contract', () => {
  test('POSTs actual consent to browser bridge, validates authorization redirect, does not claim login success', async () => {
    response = validResponse()
    await mount(supported, true)
    await act(async () => login())
    assert.deepEqual(calls, [
      {
        path: '/api/oauth/google',
        payload: { accepted_legal: true },
        options: { skipAuthRefresh: true },
      },
    ])
    assert.deepEqual(navigations, [validResponse().data.authorization_url])
    assert.equal(forbiddenCalls, 0)
    assert.deepEqual(errors, [])
  })

  test('uses the configured legacy browser callback without altering the server authorization response', async () => {
    const authorization = validResponse()
    const url = new URL(authorization.data.authorization_url)
    url.searchParams.set(
      'redirect_uri',
      'https://site.example.test/api/oauth/google'
    )
    response = { success: true, data: { authorization_url: url.toString() } }
    await mount({ ...supported, google_browser_callback: '/api/oauth/google' })
    await act(async () => login())
    assert.deepEqual(navigations, [url.toString()])
    assert.equal(forbiddenCalls, 0)
    assert.deepEqual(errors, [])
  })

  test('keeps the registered legacy SPA redirect while starting through the browser API bridge', async () => {
    const authorization = validResponse()
    const url = new URL(authorization.data.authorization_url)
    url.searchParams.set(
      'redirect_uri',
      'https://site.example.test/oauth/google'
    )
    response = { success: true, data: { authorization_url: url.toString() } }
    await mount(
      { ...supported, google_browser_callback: '/oauth/google' },
      true
    )
    await act(async () => login())
    assert.deepEqual(calls, [
      {
        path: '/api/oauth/google',
        payload: { accepted_legal: true },
        options: { skipAuthRefresh: true },
      },
    ])
    assert.deepEqual(navigations, [url.toString()])
    assert.equal(forbiddenCalls, 0)
    assert.deepEqual(errors, [])
  })

  test('legal consent failure makes no start request or navigation', async () => {
    response = validResponse()
    await mount({ ...supported, user_agreement_enabled: true }, false)
    await act(async () => login())
    assert.deepEqual(calls, [])
    assert.deepEqual(navigations, [])
    assert.equal(errors.length, 1)
  })

  test('missing native browser capability makes no request', async () => {
    response = validResponse()
    await mount({ ...supported, google_oauth: false })
    await act(async () => login())
    assert.deepEqual(calls, [])
    assert.deepEqual(navigations, [])
  })

  test('never adapts native-token JSON or follows foreign redirect as browser acceptance', async () => {
    await mount()
    for (const bad of [
      { success: true, data: { access_token: 'synthetic-native-token' } },
      { success: false, data: validResponse().data },
      {
        success: true,
        data: { authorization_url: 'https://attacker.example.test' },
      },
    ]) {
      response = bad
      await act(async () => login())
    }
    assert.equal(calls.length, 3)
    assert.deepEqual(navigations, [])
    assert.equal(errors.length, 3)
    assert.equal(forbiddenCalls, 0)
  })
})

for (const kind of ['oidc', 'custom_oauth'] as const) {
  test(`native ${kind} caller posts catalog ID and actual legal consent without legacy state/logout`, async () => {
    const selected = {
      id: '79',
      kind,
      slug: 'controlled-sso',
      name: 'Synthetic SSO',
      icon: '',
      revision: '1',
      client_id: 'synthetic-client',
      authorization_endpoint: 'https://idp.example.test/authorize',
      scopes: 'openid profile',
      qr_code_url: '',
      native_federated: true as const,
    }
    const state = `lmmfe_${'7'.repeat(64)}`
    const url = `https://idp.example.test/authorize?tenant=synthetic&scope=openid+profile&client_id=synthetic-client&redirect_uri=${encodeURIComponent(`https://site.example.test/oauth/${selected.slug}`)}&response_type=code&state=${state}&nonce=synthetic&code_challenge=${'a'.repeat(43)}&code_challenge_method=S256`
    response = {
      success: true,
      data: { authorization_url: url, state, expires_at: 2000000000 },
    }
    function NativeHarness() {
      const { handleFederatedLogin } = useOAuthLogin(
        { native_federated_auth: true, federated_oauth_providers: [selected] },
        '/dashboard',
        true
      )
      useLayoutEffect(() => {
        login = () => handleFederatedLogin(selected)
      }, [handleFederatedLogin])
      return null
    }
    const host = document.createElement('div')
    document.body.append(host)
    root = createRoot(host)
    await act(async () => root!.render(<NativeHarness />))
    await act(async () => login())
    assert.equal(calls.length, 1)
    assert.equal(calls[0].path, '/api/oauth/federated/79/begin')
    assert.deepEqual(calls[0].payload, { accepted_legal: true, affiliate: '' })
    assert.deepEqual(navigations, [url])
    assert.equal(forbiddenCalls, 0)
    assert.deepEqual(errors, [])
    const { readFederatedCeremony } = await import('../lib/federated-oauth')
    assert.equal(
      readFederatedCeremony(dom.sessionStorage, state, selected.slug, 'login')
        .providerId,
      '79'
    )
  })
}
