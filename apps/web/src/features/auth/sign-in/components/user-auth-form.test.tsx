/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
// @ts-expect-error Bun's test module is available only in the test runtime.
import { mock } from 'bun:test'
import assert from 'node:assert/strict'
import { after, afterEach, beforeEach, test } from 'node:test'

import { AxiosError, AxiosHeaders, CanceledError } from 'axios'
import { Window } from 'happy-dom'
import i18next from 'i18next'
import type { ReactNode } from 'react'

import {
  AuthCookieBlockedError,
  authCookieRecoveryMessage,
  authCookieStorageMessage,
  withAuthCookieReconciliationLock,
} from '@/lib/auth-cookie'
import { useAuthStore } from '@/stores/auth-store'
import { installSupportedAuthBrowser } from '@/test-support/supported-auth-browser'

const unexpectedRequests: string[] = []
function rejectNetwork({ request }: { request: { url: string } }): never {
  unexpectedRequests.push(request.url)
  throw new Error(
    `Unexpected network request in sign-in form test: ${request.url}`
  )
}

const dom = new Window({
  url: 'https://console.example.test/sign-in',
  settings: {
    fetch: {
      interceptor: {
        beforeAsyncRequest: async (context) => rejectNetwork(context),
        beforeSyncRequest: rejectNetwork,
      },
    },
  },
})
dom.document.write('<!doctype html><html><head></head><body></body></html>')
Object.defineProperty(dom.document, 'compatMode', {
  configurable: true,
  value: 'CSS1Compat',
})

const originalGlobals = new Map<string, PropertyDescriptor | undefined>()
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
  'MouseEvent',
  'CustomEvent',
  'MutationObserver',
  'ResizeObserver',
  'requestAnimationFrame',
  'cancelAnimationFrame',
  'getComputedStyle',
  'matchMedia',
  'customElements',
  'localStorage',
  'sessionStorage',
] as const) {
  originalGlobals.set(key, Object.getOwnPropertyDescriptor(globalThis, key))
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: dom[key],
  })
}
originalGlobals.set(
  'IS_REACT_ACT_ENVIRONMENT',
  Object.getOwnPropertyDescriptor(globalThis, 'IS_REACT_ACT_ENVIRONMENT')
)
Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
  configurable: true,
  value: true,
})

const toastErrors: string[] = []
const toastErrorIds: Array<string | undefined> = []
const toastSuccesses: string[] = []
const loginCalls: unknown[] = []
const passkeyUsernames: string[] = []
const wechatStartCalls: unknown[] = []
const oauthClicks: string[] = []
const successBundles: unknown[] = []
const pending2FATokens: string[] = []
const otpRedirects: unknown[] = []
let activeStatus: Record<string, unknown> | null = null
let loginImpl: (payload: unknown) => Promise<unknown> = async () => ({
  success: true,
  data: {
    access_token: 'test-token',
    token_type: 'Bearer',
    access_expires_at: 2000000000,
    user: { id: 1, username: 'alice', role: 1 },
    session: {
      sid: 'synthetic-password-sid',
      current: true,
      login_method: 'password',
      ip: '',
      user_agent: '',
      created_at: 1,
      last_active_at: 1,
      expires_at: 2000000000,
    },
  },
})

mock.module('sonner', () => ({
  toast: {
    error: (message: string, options?: { id?: string }) => {
      toastErrors.push(message)
      toastErrorIds.push(options?.id)
      return 'error-toast'
    },
    success: (message: string) => {
      toastSuccesses.push(message)
      return 'success-toast'
    },
    info: (message: string) => {
      toastErrors.push(message)
      return 'info-toast'
    },
  },
}))

mock.module('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string, values?: Record<string, unknown>) => {
      if (!values) return key
      return Object.entries(values).reduce(
        (message, [name, value]) =>
          message.replaceAll(`{{${name}}}`, String(value)),
        key
      )
    },
  }),
}))

mock.module('@tanstack/react-router', () => ({
  Link: ({ to, children, ...props }: { to: string; children: ReactNode }) => (
    <a href={to} {...props}>
      {children}
    </a>
  ),
}))

mock.module('@/features/auth/federated-api', () => ({
  beginFederatedLogin: async (_provider: unknown, acceptedLegal: boolean) => {
    wechatStartCalls.push(acceptedLegal)
    return {
      state: `lmmfe_${'a'.repeat(64)}`,
      expires_at: 2000000000,
      authorization_url: '',
    }
  },
  finishFederatedLogin: async () => ({ success: true, data: {} }),
  parseFederatedChallenge: () => null,
}))

mock.module('@/features/auth/api', () => ({
  login: async (payload: unknown) => {
    loginCalls.push(payload)
    return commitMockResponse(await loginImpl(payload))
  },
}))

mock.module('@/features/auth/hooks/use-auth-redirect', () => ({
  useAuthRedirect: () => ({
    handleLoginSuccess: async (bundle: unknown) => {
      successBundles.push(bundle)
    },
    redirectTo2FA: (redirect: unknown) => {
      otpRedirects.push(redirect)
    },
  }),
}))

mock.module('@/features/auth/hooks/use-turnstile', () => ({
  useTurnstile: () => ({
    isTurnstileEnabled: false,
    turnstileSiteKey: '',
    turnstileToken: 'turnstile-token',
    setTurnstileToken: () => {},
    validateTurnstile: () => true,
  }),
}))

mock.module('@/hooks/use-status', () => ({
  useStatus: () => ({ status: activeStatus }),
}))

let beginPasskeyImpl: () => Promise<unknown> = async () => ({
  success: false,
  message: 'unused',
})
let finishPasskeyImpl: () => Promise<unknown> = async () => ({
  success: false,
  message: 'unused',
})
let requestPasskeyImpl: () => Promise<Record<
  string,
  unknown
> | null> = async () => ({})
async function commitMockResponse(value: unknown) {
  const response = value as { success: boolean; data?: unknown }
  const { isAuthBundle } = await import('@/lib/auth-session')
  if (response.success && isAuthBundle(response.data)) {
    useAuthStore.getState().auth.setBundle(response.data)
  }
  const data = response.data as
    | { require_2fa?: boolean; flow_token?: string; expires_at?: number }
    | undefined
  if (response.success && data?.require_2fa && data.flow_token) {
    pending2FATokens.push(data.flow_token)
    useAuthStore
      .getState()
      .auth.setPending2FAFlowToken(data.flow_token, data.expires_at)
  }
  return response
}
mock.module('@/features/auth/passkey', () => ({
  beginPasskeyLogin: async (username: string, assertCurrent: () => void) => {
    passkeyUsernames.push(username)
    const response = await beginPasskeyImpl()
    assertCurrent()
    return response
  },
  finishPasskeyLogin: async (
    _flow: string,
    _credential: unknown,
    assertCurrent: () => void
  ) => {
    const response = await finishPasskeyImpl()
    assertCurrent()
    return commitMockResponse(response)
  },
}))

mock.module('@/lib/passkey', () => ({
  buildAssertionResult: () => ({}),
  prepareCredentialRequestOptions: (options: unknown) => options,
  isPasskeySupported: async () => true,
  requestPasskeyAuthentication: () => requestPasskeyImpl(),
}))

mock.module('@/features/auth/hooks/use-oauth-login', () => ({
  useOAuthLogin: () => ({
    isLoading: false,
    githubButtonText: 'Continue with GitHub',
    githubButtonDisabled: false,
    handleGitHubLogin: () => oauthClicks.push('github'),
    handleDiscordLogin: () => oauthClicks.push('discord'),
    handleLinuxDOLogin: () => oauthClicks.push('linuxdo'),
    handleTelegramLogin: () => oauthClicks.push('telegram'),
    handleGoogleLogin: () => oauthClicks.push('google'),
    handleFederatedLogin: (provider: { id: string }) =>
      oauthClicks.push(`federated:${provider.id}`),
    isTelegramDialogOpen: false,
    isTelegramPending: false,
    handleTelegramAuthorization: () => {},
    handleTelegramDialogChange: () => {},
  }),
}))

mock.module('@/assets/brand-icons', () => ({
  IconDiscord: () => <span aria-hidden='true' />,
  IconGithub: () => <span aria-hidden='true' />,
  IconGoogle: () => <span aria-hidden='true' />,
  IconLinuxDo: () => <span aria-hidden='true' />,
  IconTelegram: () => <span aria-hidden='true' />,
  IconWeChat: () => <span aria-hidden='true' />,
}))

mock.module('@/features/auth/components/telegram-login-dialog', () => ({
  TelegramLoginDialog: () => null,
}))

mock.module('@/features/profile/components/account-closure-recovery', () => ({
  AccountClosureRecovery: () => null,
}))

mock.module('./account-appeal-form', () => ({
  AccountAppealForm: ({ refreshKey }: { refreshKey: number }) => (
    <div data-testid='account-appeal-form' data-refresh-key={refreshKey} />
  ),
}))

const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
await i18next.init({
  lng: 'en',
  resources: { en: { translation: {} } },
})
const { UserAuthForm } = await import('./user-auth-form')

type Root = ReturnType<typeof createRoot>
let root: Root | undefined

const flush = () => new Promise((resolve) => setTimeout(resolve, 20))

async function waitFor(check: () => boolean, message: string) {
  for (let attempt = 0; attempt < 30 && !check(); attempt += 1) {
    await act(flush)
  }
  assert.ok(check(), message)
}

async function render(status: Record<string, unknown> | null) {
  activeStatus = status
  const host = document.createElement('div')
  document.body.append(host)
  const activeRoot = createRoot(host)
  root = activeRoot
  await act(async () => {
    activeRoot.render(<UserAuthForm />)
    await flush()
  })
  await act(flush)
  return host
}

function buttonByName(host: HTMLElement, name: string) {
  const button = [...host.querySelectorAll('button')].find((candidate) =>
    candidate.textContent?.includes(name)
  )
  assert.ok(button, `missing button: ${name}`)
  return button as HTMLButtonElement
}

function buttonByAccessibleName(host: HTMLElement, name: string) {
  const button = [...host.querySelectorAll('button')].find(
    (candidate) =>
      candidate.getAttribute('aria-label') === name ||
      candidate.textContent?.includes(name)
  )
  assert.ok(button, `missing button: ${name}`)
  return button as HTMLButtonElement
}

function inputByName(host: HTMLElement, name: string) {
  const input = host.querySelector(`input[name="${name}"]`)
  assert.ok(input, `missing input: ${name}`)
  return input as HTMLInputElement
}

function passwordSubmitButton(host: HTMLElement) {
  const button = host.querySelector('button[type="submit"]')
  assert.ok(button, 'missing password submit button')
  return button as HTMLButtonElement
}

const setNativeInputValue = Object.getOwnPropertyDescriptor(
  dom.HTMLInputElement.prototype,
  'value'
)?.set
assert.ok(setNativeInputValue)

async function setInputValue(input: HTMLInputElement, value: string) {
  await act(async () => {
    assert.ok(setNativeInputValue, 'native input setter is available')
    setNativeInputValue.call(input, value)
    input.dispatchEvent(new Event('input', { bubbles: true }))
    await flush()
  })
}

async function click(element: HTMLElement) {
  await act(async () => {
    element.click()
    await flush()
  })
}

async function fillCredentials(host: HTMLElement) {
  await setInputValue(inputByName(host, 'username'), 'alice@example.test')
  await setInputValue(inputByName(host, 'password'), 'correct horse')
}

async function submitPassword(host: HTMLElement) {
  await click(passwordSubmitButton(host))
}

beforeEach(() => {
  requestPasskeyImpl = async () => ({})
  otpRedirects.length = 0
  useAuthStore.getState().auth.reset('complete')
  beginPasskeyImpl = async () => ({ success: false, message: 'unused' })
  finishPasskeyImpl = async () => ({ success: false, message: 'unused' })
  activeStatus = null
  loginImpl = async () => ({
    success: true,
    data: {
      access_token: 'test-token',
      token_type: 'Bearer',
      access_expires_at: 2000000000,
      user: { id: 1, username: 'alice', role: 1 },
      session: {
        sid: 'synthetic-password-sid',
        current: true,
        login_method: 'password',
        ip: '',
        user_agent: '',
        created_at: 1,
        last_active_at: 1,
        expires_at: 2000000000,
      },
    },
  })
})

afterEach(async () => {
  const activeRoot = root
  if (activeRoot) await act(() => activeRoot.unmount())
  root = undefined
  document.body.innerHTML = ''
  toastErrors.length = 0
  toastErrorIds.length = 0
  toastSuccesses.length = 0
  loginCalls.length = 0
  passkeyUsernames.length = 0
  wechatStartCalls.length = 0
  oauthClicks.length = 0
  successBundles.length = 0
  pending2FATokens.length = 0
  localStorage.clear()
  sessionStorage.clear()
  assert.deepEqual(unexpectedRequests.splice(0), [])
})

after(() => {
  dom.close()
  for (const [key, descriptor] of originalGlobals) {
    if (descriptor) Object.defineProperty(globalThis, key, descriptor)
    else Reflect.deleteProperty(globalThis, key)
  }
})

test('missing legal consent leaves password submit clickable and uses existing toast gate', async () => {
  const host = await render({
    user_agreement_enabled: true,
    privacy_policy_enabled: true,
  })
  await fillCredentials(host)

  const submit = passwordSubmitButton(host)
  assert.equal(submit.disabled, false)

  await submitPassword(host)

  assert.deepEqual(toastErrors, ['Please agree to the legal terms first'])
  assert.equal(loginCalls.length, 0)
  assert.equal(successBundles.length, 0)
})

test('accepted legal consent keeps the existing password login path', async () => {
  const host = await render({
    user_agreement_enabled: true,
    privacy_policy_enabled: true,
  })
  await fillCredentials(host)

  const consent = host.querySelector('#legal-consent')
  assert.ok(consent, 'legal consent control is rendered')
  await click(consent as HTMLElement)

  await submitPassword(host)

  assert.deepEqual(toastErrors, [])
  assert.equal(loginCalls.length, 1)
  assert.deepEqual(loginCalls[0], {
    username: 'alice@example.test',
    password: 'correct horse',
    turnstile: 'turnstile-token',
  })
  assert.equal(successBundles.length, 1)
  assert.deepEqual(toastSuccesses, ['Welcome back!'])
})

test('without a legal requirement the password login path is unchanged', async () => {
  const host = await render({
    user_agreement_enabled: false,
    privacy_policy_enabled: false,
  })
  await fillCredentials(host)

  assert.equal(host.querySelectorAll('a[href="/forgot-password"]').length, 1)

  assert.equal(
    host.textContent?.includes('I have read and agree to the'),
    false
  )
  await submitPassword(host)

  assert.deepEqual(toastErrors, [])
  assert.equal(loginCalls.length, 1)
  assert.deepEqual(loginCalls[0], {
    username: 'alice@example.test',
    password: 'correct horse',
    turnstile: 'turnstile-token',
  })
  assert.equal(successBundles.length, 1)
})

test('password submit is disabled while login is loading', async () => {
  let resolveLogin: (value: unknown) => void = () => {}
  loginImpl = async () =>
    new Promise((resolve) => {
      resolveLogin = resolve
    })
  const host = await render({
    user_agreement_enabled: false,
    privacy_policy_enabled: false,
  })
  await fillCredentials(host)

  const submit = passwordSubmitButton(host)
  assert.equal(submit.disabled, false)
  await submitPassword(host)

  await waitFor(
    () => submit.disabled,
    'password submit did not enter loading state'
  )
  assert.equal(loginCalls.length, 1)

  await act(async () => {
    resolveLogin({
      success: true,
      data: {
        access_token: 'test-token',
        token_type: 'Bearer',
        access_expires_at: 2000000000,
        user: { id: 1, username: 'alice', role: 1 },
        session: {
          sid: 'synthetic-password-sid',
          current: true,
          login_method: 'password',
          ip: '',
          user_agent: '',
          created_at: 1,
          last_active_at: 1,
          expires_at: 2000000000,
        },
      },
    })
    await flush()
  })
  await waitFor(
    () => !submit.disabled,
    'password submit did not leave loading state'
  )
})

test('legal-consent policies for Passkey OAuth and WeChat remain disabled before consent', async () => {
  const host = await render({
    user_agreement_enabled: true,
    privacy_policy_enabled: true,
    passkey_login: true,
    wechat_login: true,
    native_federated_auth: true,
    federated_oauth_providers: [
      {
        id: '41',
        kind: 'wechat',
        slug: 'wechat-fixture',
        name: 'WeChat',
        icon: '',
        revision: '1',
        client_id: '',
        authorization_endpoint: '',
        scopes: '',
        qr_code_url: '',
        native_federated: true,
      },
    ],
    google_oauth: true,
    google_client_id: 'google-client',
  })

  await waitFor(
    () => host.textContent?.includes('Sign in with Passkey') === true,
    'passkey login did not render'
  )

  const consent = host.querySelector('#legal-consent')
  assert.ok(consent)
  const orderedControls = [
    inputByName(host, 'username'),
    inputByName(host, 'password'),
    consent,
    passwordSubmitButton(host),
    buttonByAccessibleName(host, 'Continue with Google'),
    buttonByAccessibleName(host, 'Continue with WeChat'),
    buttonByName(host, 'Sign in with Passkey'),
  ]
  for (const [index, control] of orderedControls.slice(1).entries()) {
    assert.ok(
      orderedControls[index].compareDocumentPosition(control) &
        Node.DOCUMENT_POSITION_FOLLOWING,
      'password fields and legal consent precede submit and alternative sign-in'
    )
  }
  assert.equal(passwordSubmitButton(host).disabled, false)
  assert.equal(buttonByName(host, 'Sign in with Passkey').disabled, true)
  assert.equal(
    buttonByAccessibleName(host, 'Continue with WeChat').disabled,
    true
  )
  assert.equal(
    buttonByAccessibleName(host, 'Continue with Google').disabled,
    true
  )

  await fillCredentials(host)
  await submitPassword(host)
  assert.deepEqual(toastErrors, ['Please agree to the legal terms first'])
  assert.equal(loginCalls.length, 0)
  assert.deepEqual(wechatStartCalls, [])
  assert.deepEqual(oauthClicks, [])

  await click(consent as HTMLElement)
  assert.equal(buttonByName(host, 'Sign in with Passkey').disabled, false)
  assert.equal(
    buttonByAccessibleName(host, 'Continue with WeChat').disabled,
    false
  )
  assert.equal(
    buttonByAccessibleName(host, 'Continue with Google').disabled,
    false
  )

  await click(buttonByAccessibleName(host, 'Continue with Google'))
  assert.deepEqual(oauthClicks, ['google'])
  await click(consent as HTMLElement)
  assert.equal(buttonByName(host, 'Sign in with Passkey').disabled, true)
  assert.equal(
    buttonByAccessibleName(host, 'Continue with WeChat').disabled,
    true
  )
  assert.equal(
    buttonByAccessibleName(host, 'Continue with Google').disabled,
    true
  )
  assert.deepEqual(wechatStartCalls, [])
})

for (const [terms, privacy] of [
  [true, false],
  [false, true],
  [true, true],
] as const) {
  test(`terms=${terms}, privacy=${privacy} requires explicit consent and blocks again after withdrawal`, async () => {
    const host = await render({
      user_agreement_enabled: terms,
      privacy_policy_enabled: privacy,
    })
    await fillCredentials(host)

    const consent = host.querySelector<HTMLInputElement>('#legal-consent')
    assert.ok(consent, 'consent control is rendered')
    assert.equal(consent.checked, false)
    assert.equal(
      Boolean(host.querySelector('a[href="/user-agreement"]')),
      terms
    )
    assert.equal(
      Boolean(host.querySelector('a[href="/privacy-policy"]')),
      privacy
    )
    assert.equal(passwordSubmitButton(host).disabled, false)

    await submitPassword(host)
    assert.deepEqual(toastErrors, ['Please agree to the legal terms first'])
    assert.equal(loginCalls.length, 0)
    assert.equal(successBundles.length, 0)

    await click(consent as HTMLElement)
    assert.equal(consent.checked, true)
    await submitPassword(host)
    assert.equal(loginCalls.length, 1)
    assert.equal(successBundles.length, 1)
    assert.deepEqual(toastSuccesses, ['Welcome back!'])

    await click(consent as HTMLElement)
    assert.equal(consent.checked, false)
    await submitPassword(host)
    assert.equal(
      loginCalls.length,
      1,
      'withdrawn consent must not send a request'
    )
    assert.equal(successBundles.length, 1)
    assert.deepEqual(toastErrors, [
      'Please agree to the legal terms first',
      'Please agree to the legal terms first',
    ])
  })
}

for (const status of [
  { password_login_enabled: false },
  { data: { password_login_enabled: false } },
]) {
  test(`password login disabled by ${JSON.stringify(status)} stays unavailable`, async () => {
    const host = await render({
      ...status,
      user_agreement_enabled: true,
      privacy_policy_enabled: true,
    })
    assert.equal(host.querySelector('button[type="submit"]'), null)
    assert.equal(host.querySelector('input[name="username"]'), null)
    assert.equal(host.querySelector('input[name="password"]'), null)
    assert.equal(host.querySelectorAll('a[href="/forgot-password"]').length, 1)
    const consent = host.querySelector('#legal-consent')
    assert.ok(consent)
    await click(consent as HTMLElement)
    assert.equal(host.querySelector('button[type="submit"]'), null)
    assert.equal(loginCalls.length, 0)
  })
}

test('Passkey-only login keeps the username for non-discoverable credentials without password submission', async () => {
  Object.defineProperty(navigator, 'credentials', {
    configurable: true,
    value: { get: async () => ({ id: 'synthetic-passkey' }) },
  })
  const host = await render({
    password_login_enabled: false,
    passkey_login: true,
  })
  assert.equal(host.querySelector('input[name="password"]'), null)
  await setInputValue(inputByName(host, 'username'), 'security-key-user')
  beginPasskeyImpl = async () => ({
    success: false,
    message: 'synthetic rejection',
  })
  await click(buttonByName(host, 'Sign in with Passkey'))
  assert.deepEqual(passkeyUsernames, ['security-key-user'])
  await act(async () => {
    const form = host.querySelector('form')
    assert.ok(form)
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
    await flush()
  })
  assert.equal(loginCalls.length, 0)
})

function loginHttpError(
  status: number,
  data: unknown,
  headers: Record<string, string> = {}
) {
  return new AxiosError(
    'private transport diagnostic',
    'ERR_BAD_RESPONSE',
    undefined,
    undefined,
    {
      status,
      statusText: 'synthetic failure',
      config: { headers: new AxiosHeaders() },
      data,
      headers,
    }
  )
}

for (const [name, error, expectedErrors] of [
  [
    'unknown client diagnostic',
    new Error('private service diagnostic'),
    ['Login failed'],
  ],
  ['unknown failure', 'unknown failure', ['Login failed']],
  [
    'network failure',
    new AxiosError('private network diagnostic'),
    ['Login failed'],
  ],
  [
    'invalid password',
    loginHttpError(401, {
      code: 'AUTH_UNAUTHORIZED',
      message: 'Invalid credentials or expired session',
    }),
    ['Login failed'],
  ],
  [
    '401 with unrelated session code',
    loginHttpError(401, {
      code: 'AUTH_ROTATION_INVALID',
      message: 'private diagnostic',
    }),
    ['Login failed'],
  ],
  [
    'origin denied',
    loginHttpError(403, {
      code: 'AUTH_ORIGIN_FORBIDDEN',
      message: 'private origin diagnostic',
    }),
    ['Request failed'],
  ],
  [
    '403 with unrelated session code',
    loginHttpError(403, {
      code: 'AUTH_ROTATION_INVALID',
      message: 'private diagnostic',
    }),
    ['Request failed'],
  ],
  [
    'disabled account',
    loginHttpError(403, {
      code: 'AUTH_USER_DISABLED',
      message: 'private account diagnostic',
    }),
    ['This user account is disabled.'],
  ],
  [
    'rate limit with wait',
    loginHttpError(
      429,
      { message: 'private limiter diagnostic' },
      { 'retry-after': '12' }
    ),
    ['Too many requests. Please try again in 12 seconds.'],
  ],
  [
    'rate limit without valid wait',
    loginHttpError(
      429,
      { message: 'private limiter diagnostic' },
      { 'retry-after': 'invalid' }
    ),
    ['Too many requests. Please wait before trying again.'],
  ],
  [
    'wrapped rate limit',
    new Error('private wrapper diagnostic', {
      cause: loginHttpError(429, {}, { 'retry-after': '3' }),
    }),
    ['Too many requests. Please try again in 3 seconds.'],
  ],
  [
    'authentication unavailable',
    loginHttpError(503, {
      code: 'AUTH_UNAVAILABLE',
      message: 'private storage diagnostic',
    }),
    ['The service is temporarily unavailable. Please try again later.'],
  ],
  [
    'MFA unavailable',
    loginHttpError(503, {
      code: 'MFA_UNAVAILABLE',
      message: 'private MFA diagnostic',
    }),
    ['Two-factor authentication is temporarily unavailable. Try again later.'],
  ],
  [
    'MFA expired',
    Object.assign(new Error('private diagnostic'), {
      code: 'MFA_FLOW_EXPIRED',
    }),
    ['This login flow has expired or has already been used. Sign in again.'],
  ],
  [
    'invalid MFA code',
    loginHttpError(400, {
      error: { code: 'MFA_CODE_INVALID', message: 'private diagnostic' },
    }),
    ['Invalid code'],
  ],
  [
    'unknown backend error',
    loginHttpError(500, {
      code: 'UNKNOWN_FAILURE',
      message: 'private backend diagnostic',
    }),
    ['Login failed'],
  ],
  [
    'Cookie result uncertain',
    new AuthCookieBlockedError('uncertain'),
    [authCookieRecoveryMessage],
  ],
  [
    'Cookie storage blocked',
    new AuthCookieBlockedError('storage_unavailable'),
    [authCookieStorageMessage],
  ],
  ['cancelled login', new CanceledError('Login superseded'), []],
] as const) {
  test(`${name} clears loading without bypassing consent on retry`, async () => {
    let rejectLogin: (error: unknown) => void = () => {}
    loginImpl = async () =>
      new Promise((_resolve, reject) => {
        rejectLogin = reject
      })
    const host = await render({
      user_agreement_enabled: true,
      privacy_policy_enabled: true,
    })
    await fillCredentials(host)
    const consent = host.querySelector('#legal-consent')
    assert.ok(consent)
    await click(consent as HTMLElement)
    const submit = passwordSubmitButton(host)
    await submitPassword(host)
    await waitFor(() => submit.disabled, 'submit did not enter loading state')
    await submitPassword(host)
    assert.equal(loginCalls.length, 1, 'disabled submit must not repeat login')

    await act(async () => {
      rejectLogin(error)
      await flush()
    })
    await waitFor(
      () => !submit.disabled,
      'submit stayed disabled after failure'
    )
    assert.equal(toastErrors.includes('Session expired!'), false)
    assert.equal(
      toastErrors.some((message) => message.includes('private')),
      false
    )
    assert.deepEqual(toastErrors, [...expectedErrors])
    if (name.startsWith('rate limit') || name === 'wrapped rate limit') {
      assert.deepEqual(toastErrorIds, ['request-rate-limited'])
    }
    assert.deepEqual(toastSuccesses, [])
    assert.equal(successBundles.length, 0)

    await click(consent as HTMLElement)
    await submitPassword(host)
    assert.equal(loginCalls.length, 1)
    assert.deepEqual(toastErrors, [
      ...expectedErrors,
      'Please agree to the legal terms first',
    ])

    loginImpl = async () => ({
      success: true,
      data: {
        access_token: 'test-token',
        token_type: 'Bearer',
        access_expires_at: 2000000000,
        user: { id: 1, username: 'alice', role: 1 },
        session: {
          sid: 'synthetic-password-sid',
          current: true,
          login_method: 'password',
          ip: '',
          user_agent: '',
          created_at: 1,
          last_active_at: 1,
          expires_at: 2000000000,
        },
      },
    })
    await click(consent as HTMLElement)
    await submitPassword(host)
    assert.equal(loginCalls.length, 2)
    assert.equal(successBundles.length, 1)
    assert.deepEqual(toastSuccesses, ['Welcome back!'])
    assert.equal(submit.disabled, false)
  })
}

for (const [name, response, expectedMessage] of [
  [
    'unknown business failure',
    { success: false, message: 'private backend diagnostic' },
    'Login failed',
  ],
  [
    'allowlisted business failure',
    {
      success: false,
      code: 'AUTH_USER_DISABLED',
      message: 'private account diagnostic',
    },
    'This user account is disabled.',
  ],
  [
    'business failure with unrelated session code',
    {
      success: false,
      code: 'AUTH_ROTATION_INVALID',
      message: 'private diagnostic',
    },
    'Login failed',
  ],
  [
    'incomplete MFA challenge',
    { success: true, data: { require_2fa: true } },
    'Login flow expired. Please sign in again.',
  ],
] as const) {
  test(`${name} shows one safe local notification`, async () => {
    loginImpl = async () => response
    const host = await render({})
    await fillCredentials(host)
    await submitPassword(host)
    assert.deepEqual(toastErrors, [expectedMessage])
    assert.deepEqual(toastSuccesses, [])
    assert.deepEqual(successBundles, [])
    assert.equal(passwordSubmitButton(host).disabled, false)
  })
}

test('verified disabled password login refreshes the Cookie appeal context without completing login', async () => {
  loginImpl = async () => {
    throw loginHttpError(403, {
      code: 'AUTH_USER_DISABLED',
      message: 'PRIVATE diagnostic',
      data: {
        appeal_available: true,
        username: 'alice',
        public_reason: null,
        public_reason_available: false,
        expires_at: Math.floor(Date.now() / 1000) + 600,
      },
    })
  }
  const host = await render({})
  await fillCredentials(host)
  await submitPassword(host)
  assert.equal(
    host
      .querySelector('[data-testid="account-appeal-form"]')
      ?.getAttribute('data-refresh-key'),
    '1'
  )
  assert.deepEqual(successBundles, [])
  assert.deepEqual(pending2FATokens, [])
  assert.deepEqual(toastErrors, [
    'Submit an appeal using your verified identity. No local password is needed.',
  ])
})

for (const stage of ['begin', 'finish'] as const) {
  test(`disabled sign-in Passkey ${stage} refreshes the verified appeal form instead of completing login`, async () => {
    let finishes = 0
    const disabled = () =>
      loginHttpError(403, {
        code: 'AUTH_USER_DISABLED',
        message: 'PRIVATE diagnostic',
        data: {
          appeal_available: true,
          username: 'passkey-user',
          public_reason: null,
          public_reason_available: false,
          expires_at: Math.floor(Date.now() / 1000) + 600,
        },
      })
    beginPasskeyImpl = async () => {
      if (stage === 'begin') throw disabled()
      return {
        success: true,
        data: { options: { challenge: 'AQI' }, flow_token: 'passkey-flow' },
      }
    }
    finishPasskeyImpl = async () => {
      finishes++
      throw disabled()
    }
    Object.defineProperty(navigator, 'credentials', {
      configurable: true,
      value: { get: async () => ({ id: 'synthetic-passkey' }) },
    })
    const host = await render({ passkey_login: true })
    await waitFor(
      () => !buttonByName(host, 'Sign in with Passkey').disabled,
      'Passkey is unavailable'
    )
    await click(buttonByName(host, 'Sign in with Passkey'))
    assert.equal(
      host
        .querySelector('[data-testid="account-appeal-form"]')
        ?.getAttribute('data-refresh-key'),
      '1'
    )
    assert.equal(finishes, stage === 'finish' ? 1 : 0)
    assert.deepEqual(successBundles, [])
    assert.deepEqual(pending2FATokens, [])
    assert.deepEqual(toastSuccesses, [])
    assert.deepEqual(toastErrors, [
      'Submit an appeal using your verified identity. No local password is needed.',
    ])
  })
}

test('password native TOTP uses the shared completion without accepting a session bundle', async () => {
  const token = `lmmf_${'a'.repeat(64)}`
  loginImpl = async () => ({
    success: true,
    data: { require_2fa: true, flow_token: token, expires_at: 2000000000 },
  })
  const host = await render({})
  await fillCredentials(host)
  await submitPassword(host)
  assert.equal(useAuthStore.getState().auth.pending2FAFlowToken, token)
  assert.deepEqual(otpRedirects, [undefined])
  assert.deepEqual(successBundles, [])
  assert.deepEqual(toastSuccesses, [])
})
for (const outcome of ['session', 'mfa', 'cancel'] as const) {
  test(`direct Passkey shared completion preserves ${outcome} without duplicate finishing`, async () => {
    const token = `lmmf_${'b'.repeat(64)}`
    let finishes = 0
    beginPasskeyImpl = async () => ({
      success: true,
      data: {
        flow_token: `lmmp_${'c'.repeat(64)}`,
        options: { challenge: 'AQID' },
        expires_at: 2000000000,
      },
    })
    const nativeBundle = {
      access_token: 'direct-passkey-token',
      token_type: 'Bearer',
      access_expires_at: 2000000000,
      user: { id: 1, username: 'alice', role: 1 },
      session: {
        sid: 'direct-passkey-sid',
        current: true,
        login_method: 'passkey',
        ip: '',
        user_agent: '',
        created_at: 1,
        last_active_at: 1,
        expires_at: 2000000000,
      },
    }
    finishPasskeyImpl = async () => {
      finishes++
      return {
        success: true,
        data:
          outcome === 'mfa'
            ? { require_2fa: true, flow_token: token, expires_at: 2000000000 }
            : nativeBundle,
      }
    }
    if (outcome === 'cancel') requestPasskeyImpl = async () => null
    Object.defineProperty(navigator, 'credentials', {
      configurable: true,
      value: { get: async () => ({ id: 'synthetic-credential' }) },
    })
    const host = await render({ passkey_login: true })
    await waitFor(
      () => !buttonByName(host, 'Sign in with Passkey').disabled,
      'Passkey unavailable'
    )
    await click(buttonByName(host, 'Sign in with Passkey'))
    assert.equal(finishes, outcome === 'cancel' ? 0 : 1)
    assert.equal(successBundles.length, outcome === 'session' ? 1 : 0)
    assert.deepEqual(otpRedirects, outcome === 'mfa' ? [undefined] : [])
    assert.deepEqual(
      toastSuccesses,
      outcome === 'session' ? ['Signed in with Passkey'] : []
    )
    if (outcome === 'mfa') {
      assert.equal(useAuthStore.getState().auth.pending2FAFlowToken, token)
    }
    if (outcome === 'cancel') {
      assert.deepEqual(toastErrors, ['Passkey login was cancelled'])
    }
  })
}

for (const stage of [
  'begin',
  'credential',
  'finish',
  'credential-error',
] as const) {
  test(`unmount during direct Passkey ${stage} discards late results without login or presentation effects`, async () => {
    let release: (() => void) | undefined
    const pending = new Promise<void>((resolve) => {
      release = resolve
    })
    let credentials = 0,
      finishes = 0
    beginPasskeyImpl = async () => {
      if (stage === 'begin') await pending
      return {
        success: true,
        data: {
          flow_token: `lmmp_${'a'.repeat(64)}`,
          options: { challenge: 'AQID' },
          expires_at: 2000000000,
        },
      }
    }
    requestPasskeyImpl = async () => {
      credentials++
      if (stage === 'credential' || stage === 'credential-error') await pending
      if (stage === 'credential-error') {
        throw new DOMException('Synthetic browser timeout', 'NotAllowedError')
      }
      return {}
    }
    finishPasskeyImpl = async () => {
      finishes++
      await pending
      return {
        success: true,
        data: {
          access_token: 'late-passkey-access',
          token_type: 'Bearer',
          access_expires_at: 2000000000,
          user: { id: 951, username: 'synthetic', role: 1 },
          session: {
            sid: 'late-passkey-session',
            current: true,
            login_method: 'passkey',
            ip: '',
            user_agent: '',
            created_at: 1,
            last_active_at: 1,
            expires_at: 2000000000,
          },
        },
      }
    }
    Object.defineProperty(navigator, 'credentials', {
      configurable: true,
      value: { get: async () => ({}) },
    })
    const host = await render({ passkey_login: true })
    await waitFor(
      () => !buttonByName(host, 'Sign in with Passkey').disabled,
      'Passkey unavailable'
    )
    await click(buttonByName(host, 'Sign in with Passkey'))
    assert.equal(credentials, stage === 'begin' ? 0 : 1)
    assert.equal(finishes, stage === 'finish' ? 1 : 0)
    assert.ok(root)
    await act(() => root?.unmount())
    root = undefined
    await act(async () => {
      release?.()
      await flush()
    })
    assert.equal(finishes, stage === 'finish' ? 1 : 0)
    assert.equal(useAuthStore.getState().auth.user, null)
    assert.deepEqual(successBundles, [])
    assert.deepEqual(otpRedirects, [])
    assert.deepEqual(toastSuccesses, [])
    assert.deepEqual(toastErrors, [])
  })
}

test('pending direct Passkey owns the login slot against duplicate and password submissions', async () => {
  let begins = 0,
    requests = 0,
    finishes = 0
  let release: (() => void) | undefined
  const pending = new Promise<void>((resolve) => {
    release = resolve
  })
  beginPasskeyImpl = async () => {
    begins++
    return {
      success: true,
      data: {
        flow_token: `lmmp_${'b'.repeat(64)}`,
        options: { challenge: 'AQID' },
      },
    }
  }
  requestPasskeyImpl = async () => {
    requests++
    await pending
    return null
  }
  finishPasskeyImpl = async () => {
    finishes++
    return { success: false }
  }
  Object.defineProperty(navigator, 'credentials', {
    configurable: true,
    value: { get: async () => ({}) },
  })
  const host = await render({ passkey_login: true })
  await fillCredentials(host)
  await waitFor(
    () => !buttonByName(host, 'Sign in with Passkey').disabled,
    'Passkey unavailable'
  )
  const button = buttonByName(host, 'Sign in with Passkey')
  await act(async () => {
    button.click()
    button.click()
    await flush()
  })
  await submitPassword(host)
  assert.equal(begins, 1)
  assert.equal(requests, 1)
  assert.equal(finishes, 0)
  assert.deepEqual(loginCalls, [])
  await act(async () => {
    release?.()
    await flush()
  })
  assert.equal(finishes, 0)
  assert.deepEqual(successBundles, [])
  assert.deepEqual(toastErrors, ['Passkey login was cancelled'])
  assert.equal(buttonByName(host, 'Sign in with Passkey').disabled, false)
})

test('a late Passkey finish from the unmounted form cannot replace the new form password session', async () => {
  let release: (() => void) | undefined
  const pending = new Promise<void>((resolve) => {
    release = resolve
  })
  let finishes = 0
  beginPasskeyImpl = async () => ({
    success: true,
    data: {
      flow_token: `lmmp_${'e'.repeat(64)}`,
      options: { challenge: 'AQID' },
    },
  })
  finishPasskeyImpl = async () => {
    finishes++
    await pending
    return {
      success: true,
      data: {
        access_token: 'superseded-passkey-access',
        token_type: 'Bearer',
        access_expires_at: 2000000000,
        user: { id: 952, username: 'superseded', role: 1 },
        session: {
          sid: 'superseded-passkey-sid',
          current: true,
          login_method: 'passkey',
          ip: '',
          user_agent: '',
          created_at: 1,
          last_active_at: 1,
          expires_at: 2000000000,
        },
      },
    }
  }
  Object.defineProperty(navigator, 'credentials', {
    configurable: true,
    value: { get: async () => ({}) },
  })
  const oldHost = await render({ passkey_login: true })
  await waitFor(
    () => !buttonByName(oldHost, 'Sign in with Passkey').disabled,
    'Passkey unavailable'
  )
  await click(buttonByName(oldHost, 'Sign in with Passkey'))
  assert.equal(finishes, 1)
  await act(() => root?.unmount())
  root = undefined
  const host = await render({})
  await fillCredentials(host)
  await submitPassword(host)
  const current = useAuthStore.getState().auth
  assert.equal(successBundles.length, 1)
  assert.notEqual(current.user?.id, 952)
  await act(async () => {
    release?.()
    await flush()
  })
  assert.equal(useAuthStore.getState().auth.session?.sid, current.session?.sid)
  assert.equal(useAuthStore.getState().auth.accessToken, current.accessToken)
  assert.equal(successBundles.length, 1)
  assert.deepEqual(otpRedirects, [])
  assert.deepEqual(toastErrors, [])
  assert.deepEqual(toastSuccesses, ['Welcome back!'])
})

test('Cookie recovery blocks login controls and live handlers until confirmed recovery', async () => {
  const restoreBrowser = installSupportedAuthBrowser({ window: dom })
  const markerKey = 'new-api:auth-cookie:uncertain'
  const marker = 'uncertain:sign-in-form-fixture'
  try {
    Object.defineProperty(navigator, 'credentials', {
      configurable: true,
      value: { get: async () => ({}) },
    })
    const host = await render({
      passkey_login: true,
      google_oauth: true,
      google_client_id: 'google-client',
    })
    await fillCredentials(host)
    const passkey = buttonByName(host, 'Sign in with Passkey')
    await waitFor(() => !passkey.disabled, 'Passkey unavailable')
    const submit = passwordSubmitButton(host)
    const google = buttonByAccessibleName(host, 'Continue with Google')
    const form = host.querySelector('form')
    assert.ok(form)
    assert.equal(submit.disabled, false)
    assert.equal(google.disabled, false)

    // The shared fence can change before its storage notification reaches React.
    await act(async () => {
      localStorage.setItem(markerKey, marker)
      passkey.click()
      form.dispatchEvent(
        new Event('submit', { bubbles: true, cancelable: true })
      )
      await flush()
    })
    assert.deepEqual(passkeyUsernames, [])
    assert.deepEqual(loginCalls, [])
    assert.deepEqual(successBundles, [])
    assert.deepEqual(toastErrors, [])

    await act(async () => {
      dom.dispatchEvent(new dom.StorageEvent('storage', { key: markerKey }))
      await flush()
    })
    assert.equal(submit.disabled, true)
    assert.equal(passkey.disabled, true)
    assert.equal(google.disabled, true)
    await submitPassword(host)
    await click(google)
    assert.deepEqual(loginCalls, [])
    assert.deepEqual(oauthClicks, [])

    await act(async () => {
      await withAuthCookieReconciliationLock(marker, async (confirm) => {
        confirm(() => {})
      })
      await flush()
    })
    assert.equal(submit.disabled, false)
    assert.equal(passkey.disabled, false)
    assert.equal(google.disabled, false)
    await submitPassword(host)
    assert.equal(loginCalls.length, 1)
    assert.equal(successBundles.length, 1)
    assert.deepEqual(toastSuccesses, ['Welcome back!'])
  } finally {
    if (localStorage.getItem(markerKey) === marker) {
      await act(async () => {
        await withAuthCookieReconciliationLock(marker, async (confirm) => {
          confirm(() => {})
        })
      })
    }
    restoreBrowser()
  }
})
