/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
// @ts-expect-error Bun's test module is available only in the test runtime.
import { mock } from 'bun:test'
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { Window } from 'happy-dom'
import type { ReactNode } from 'react'

import { installSupportedAuthBrowser } from '@/test-support/supported-auth-browser'

const dom = new Window({ url: 'https://web.example.test/sign-in' })
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLInputElement',
  'HTMLButtonElement',
  'Element',
  'SVGElement',
  'Node',
  'Event',
  'MouseEvent',
  'CustomEvent',
  'MutationObserver',
  'ResizeObserver',
  'requestAnimationFrame',
  'cancelAnimationFrame',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: dom[key],
  })
}
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
mock.module('react-i18next', () => ({
  useTranslation: () => ({ t: (value: string) => value }),
}))
const errors: string[] = []
const infos: string[] = []
mock.module('sonner', () => ({
  toast: {
    success: () => {},
    info: (value: string) => infos.push(value),
    error: (value: string) => errors.push(value),
  },
}))
mock.module('@tanstack/react-router', () => ({
  Link: ({ children }: { children: ReactNode }) => <a>{children}</a>,
  useNavigate: () => () => {},
}))
mock.module('@/components/dialog', () => ({
  Dialog: ({
    open,
    children,
    footer,
  }: {
    open: boolean
    children: ReactNode
    footer: ReactNode
  }) =>
    open ? (
      <div role='dialog'>
        {children}
        {footer}
      </div>
    ) : null,
}))
mock.module('@/features/auth/components/humangrant-button', () => ({
  HumanGrantButton: () => null,
}))
mock.module('@/features/profile/components/account-closure-recovery', () => ({
  AccountClosureRecovery: () => null,
}))
mock.module('@/features/auth/sign-in/components/account-appeal-form', () => ({
  AccountAppealForm: () => null,
}))
const provider = {
  id: '71',
  kind: 'wechat' as const,
  slug: 'synthetic-wechat',
  name: 'WeChat',
  icon: '',
  revision: '1',
  client_id: '',
  authorization_endpoint: '',
  scopes: '',
  qr_code_url: '',
  native_federated: true as const,
  registration_enabled: true,
}
const status = {
  native_federated_auth: true,
  federated_oauth_providers: [provider],
  password_login_enabled: false,
  password_register_enabled: false,
  oauth_register_enabled: true,
  register_enabled: true,
}
mock.module('@/hooks/use-status', () => ({
  useStatus: () => ({ status, capabilitiesReady: true }),
}))
mock.module('@/features/auth/hooks/use-turnstile', () => ({
  useTurnstile: () => ({
    isTurnstileEnabled: false,
    turnstileToken: '',
    validateTurnstile: () => true,
    setTurnstileToken: () => {},
  }),
}))
mock.module('@/features/auth/hooks/use-email-verification', () => ({
  useEmailVerification: () => ({
    isSending: false,
    secondsLeft: 0,
    isActive: false,
    sendCode: async () => false,
  }),
}))
const redirects: string[] = []
mock.module('@/features/auth/hooks/use-auth-redirect', () => ({
  useAuthRedirect: () => ({
    redirectTo2FA: () => redirects.push('otp'),
    handleLoginSuccess: async () => redirects.push('signed-in'),
    redirectToLogin: () => {},
  }),
}))
mock.module('@/features/auth/components/oauth-providers', () => ({
  OAuthProviders: ({
    onWeChatLogin,
    disabled,
  }: {
    onWeChatLogin?: (p: typeof provider) => void
    disabled: boolean
  }) => (
    <button
      type='button'
      disabled={disabled}
      onClick={() => onWeChatLogin?.(provider)}
    >
      Continue with WeChat
    </button>
  ),
}))
// Normal supported browser; install after this fixture replaces its globals.
installSupportedAuthBrowser()

let passkeyResponse: () => Promise<unknown> = async () => ({
  id: 'synthetic-credential',
  type: 'public-key',
})
const realPasskey = { ...(await import('@/lib/passkey')) }
mock.module('@/lib/passkey', () => ({
  ...realPasskey,
  requestPasskeyAuthentication: () => passkeyResponse(),
  isPasskeySupported: async () => false,
}))
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { AuthCookieRecovery } = await import('@/components/auth-cookie-recovery')
const { clearAuthentication } = await import('@/lib/auth-session')
const { UserAuthForm } = await import('./sign-in/components/user-auth-form')
const { SignUpForm } = await import('./sign-up/components/sign-up-form')
const originalAdapter = api.defaults.adapter
let root: ReturnType<typeof createRoot> | undefined
async function mount(Form: typeof UserAuthForm | typeof SignUpForm) {
  const host = document.createElement('div')
  document.body.append(host)
  const mounted = createRoot(host)
  root = mounted
  await act(async () => {
    mounted.render(
      <>
        <AuthCookieRecovery />
        <Form />
      </>
    )
  })
  return host
}
function button(host: HTMLElement, name: string) {
  const found = Array.from(host.querySelectorAll('button')).find(
    (b) => b.textContent === name
  )
  assert.ok(found)
  return found
}
afterEach(async () => {
  await act(async () => root?.unmount())
  root = undefined
  document.body.replaceChildren()
  dom.sessionStorage.clear()
  dom.localStorage.clear()
  redirects.length = 0
  errors.length = 0
  infos.length = 0
  passkeyResponse = async () => ({
    id: 'synthetic-credential',
    type: 'public-key',
  })
  api.defaults.adapter = originalAdapter
  clearAuthentication(false, 'idle')
})
after(() => dom.close())
for (const [name, Form] of [
  ['sign-in', UserAuthForm],
  ['sign-up', SignUpForm],
] as const) {
  test(`${name} WeChat confirmation enters native TOTP instead of treating the challenge as a session`, async () => {
    const paths: string[] = []
    const state = `lmmfe_${'b'.repeat(64)}`
    const token = `lmmf_${'c'.repeat(64)}`
    api.defaults.adapter = async (config) => {
      assert.ok(config.url)
      paths.push(config.url)
      return {
        config,
        status: 200,
        statusText: 'OK',
        headers: {},
        data: {
          success: true,
          data: config.url.endsWith('/begin')
            ? { authorization_url: '', state, expires_at: 2000000000 }
            : { require_2fa: true, flow_token: token, expires_at: 2000000000 },
        },
      }
    }
    const host = await mount(Form)
    await act(async () => button(host, 'Continue with WeChat').click())
    const input = host.querySelector<HTMLInputElement>('#wechat-code')
    assert.ok(input)
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(
        dom.HTMLInputElement.prototype,
        'value'
      )?.set
      assert.ok(setter)
      setter.call(input, 'synthetic-code')
      input.dispatchEvent(new Event('input', { bubbles: true }))
    })
    await act(async () => button(host, 'Confirm').click())
    assert.deepEqual(redirects, ['otp'])
    assert.equal(useAuthStore.getState().auth.pending2FAFlowToken, token)
    assert.equal(useAuthStore.getState().auth.user, null)
    assert.deepEqual(paths, [
      '/api/oauth/federated/71/begin',
      '/api/oauth/federated/71/finish',
    ])
    assert.deepEqual(errors, [])
  })
}

for (const [name, Form] of [
  ['sign-in', UserAuthForm],
  ['sign-up', SignUpForm],
] as const) {
  test(`${name} WeChat passkey requirement uses existing passkey finish and then native TOTP`, async () => {
    const paths: string[] = []
    const state = `lmmfe_${'d'.repeat(64)}`
    const passkey = `lmmp_${'e'.repeat(64)}`
    const totp = `lmmf_${'f'.repeat(64)}`
    api.defaults.adapter = async (config) => {
      assert.ok(config.url)
      paths.push(config.url)
      const data = config.url.endsWith('/begin')
        ? { authorization_url: '', state, expires_at: 2000000000 }
        : config.url.includes('/federated/')
          ? {
              require_passkey: true,
              flow_token: passkey,
              expires_at: 2000000000,
              options: { publicKey: { challenge: 'AQID' } },
            }
          : { require_2fa: true, flow_token: totp, expires_at: 2000000000 }
      return {
        config,
        status: 200,
        statusText: 'OK',
        headers: {},
        data: { success: true, data },
      }
    }
    const host = await mount(Form)
    await act(async () => button(host, 'Continue with WeChat').click())
    const input = host.querySelector<HTMLInputElement>('#wechat-code')
    assert.ok(input)
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(
        dom.HTMLInputElement.prototype,
        'value'
      )?.set
      assert.ok(setter)
      setter.call(input, 'synthetic-code')
      input.dispatchEvent(new Event('input', { bubbles: true }))
    })
    await act(async () => button(host, 'Confirm').click())
    assert.deepEqual(redirects, ['otp'])
    assert.equal(useAuthStore.getState().auth.pending2FAFlowToken, totp)
    assert.deepEqual(paths, [
      '/api/oauth/federated/71/begin',
      '/api/oauth/federated/71/finish',
      '/api/user/passkey/login/finish',
    ])
    assert.deepEqual(errors, [])
  })
}

for (const [name, Form] of [
  ['sign-in', UserAuthForm],
  ['sign-up', SignUpForm],
] as const) {
  test(`${name} WeChat completes through the shared handler without applying the same session twice`, async () => {
    const state = `lmmfe_${'8'.repeat(64)}`
    const bundle = {
      access_token: 'synthetic-session-access',
      token_type: 'Bearer',
      access_expires_at: 2000000000,
      user: { id: 71, username: 'synthetic', role: 1 },
      session: {
        sid: 'synthetic-sid',
        current: true,
        login_method: 'wechat',
        ip: '',
        user_agent: '',
        created_at: 1,
        last_active_at: 1,
        expires_at: 2000000000,
      },
    }
    api.defaults.adapter = async (config) => {
      assert.ok(config.url)
      return {
        config,
        status: 200,
        statusText: 'OK',
        headers: {},
        data: {
          success: true,
          data: config.url.endsWith('/begin')
            ? { authorization_url: '', state, expires_at: 2000000000 }
            : bundle,
        },
      }
    }
    const host = await mount(Form)
    await act(async () => button(host, 'Continue with WeChat').click())
    const input = host.querySelector<HTMLInputElement>('#wechat-code')
    assert.ok(input)
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(
        dom.HTMLInputElement.prototype,
        'value'
      )?.set
      assert.ok(setter)
      setter.call(input, 'synthetic-code')
      input.dispatchEvent(new Event('input', { bubbles: true }))
    })
    const initial = useAuthStore.getState().auth.accountWriteEpoch
    await act(async () => button(host, 'Confirm').click())
    assert.equal(useAuthStore.getState().auth.accountWriteEpoch, initial + 1)
    assert.deepEqual(redirects, ['signed-in'])
    assert.equal(useAuthStore.getState().auth.session?.sid, 'synthetic-sid')
    assert.deepEqual(errors, [])
  })
}

for (const result of ['null', 'NotAllowed'] as const) {
  test(`WeChat Passkey ${result} is one info cancellation without finish/login`, async () => {
    const paths: string[] = []
    passkeyResponse = async () => {
      if (result === 'NotAllowed') {
        throw new DOMException('Cancelled', 'NotAllowedError')
      }
      return null
    }
    api.defaults.adapter = async (config) => {
      paths.push(String(config.url))
      assert.ok(!String(config.url).includes('/passkey/'))
      return {
        config,
        status: 200,
        statusText: 'OK',
        headers: {},
        data: {
          success: true,
          data: String(config.url).endsWith('/begin')
            ? {
                authorization_url: '',
                state: `lmmfe_${'d'.repeat(64)}`,
                expires_at: 2000000000,
              }
            : {
                require_passkey: true,
                flow_token: `lmmp_${'e'.repeat(64)}`,
                expires_at: 2000000000,
                options: { publicKey: { challenge: 'AQID' } },
              },
        },
      }
    }
    const host = await mount(UserAuthForm)
    await act(async () => button(host, 'Continue with WeChat').click())
    const input = host.querySelector<HTMLInputElement>('#wechat-code')
    assert.ok(input)
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(
        dom.HTMLInputElement.prototype,
        'value'
      )?.set
      assert.ok(setter)
      setter.call(input, 'synthetic-code')
      input.dispatchEvent(new Event('input', { bubbles: true }))
    })
    await act(async () => button(host, 'Confirm').click())
    assert.deepEqual(infos, ['Passkey login was cancelled'])
    assert.deepEqual(errors, [])
    assert.equal(paths.length, 2)
    assert.deepEqual(redirects, [])
    assert.equal(useAuthStore.getState().auth.user, null)
  })
}

test('unmount while WeChat Passkey is pending discards late credential without finish/login/notification', async () => {
  const paths: string[] = []
  let resolve!: (value: unknown) => void
  passkeyResponse = () =>
    new Promise((done) => {
      resolve = done
    })
  api.defaults.adapter = async (config) => {
    paths.push(String(config.url))
    assert.ok(!String(config.url).includes('/passkey/'))
    return {
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: {
        success: true,
        data: String(config.url).endsWith('/begin')
          ? {
              authorization_url: '',
              state: `lmmfe_${'d'.repeat(64)}`,
              expires_at: 2000000000,
            }
          : {
              require_passkey: true,
              flow_token: `lmmp_${'e'.repeat(64)}`,
              expires_at: 2000000000,
              options: { publicKey: { challenge: 'AQID' } },
            },
      },
    }
  }
  const host = await mount(UserAuthForm)
  await act(async () => button(host, 'Continue with WeChat').click())
  const input = host.querySelector<HTMLInputElement>('#wechat-code')
  assert.ok(input)
  await act(async () => {
    const setter = Object.getOwnPropertyDescriptor(
      dom.HTMLInputElement.prototype,
      'value'
    )?.set
    assert.ok(setter)
    setter.call(input, 'synthetic-code')
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
  await act(async () => button(host, 'Confirm').click())
  assert.equal(paths.length, 2)
  await act(async () => {
    root?.unmount()
    root = undefined
  })
  await act(async () => resolve({ id: 'late-credential', type: 'public-key' }))
  assert.equal(paths.length, 2)
  assert.deepEqual(errors, [])
  assert.deepEqual(infos, [])
  assert.deepEqual(redirects, [])
  assert.equal(useAuthStore.getState().auth.user, null)
})

test('real WeChat begin Cookie guard leaves exactly the root recovery alert, no toast or login', async () => {
  let writes = 0
  api.defaults.adapter = async () => {
    writes++
    throw new Error('must not reach HTTP')
  }
  const host = await mount(UserAuthForm)
  Object.defineProperty(navigator, 'locks', {
    configurable: true,
    value: undefined,
  })
  await act(async () => button(host, 'Continue with WeChat').click())
  assert.equal(writes, 0)
  assert.equal(document.querySelectorAll('[role="alert"]').length, 1)
  assert.match(document.body.textContent || '', /End this tab's local session/)
  assert.match(
    document.body.textContent || '',
    /Server sign-out is not confirmed/
  )
  assert.deepEqual(errors, [])
  assert.deepEqual(infos, [])
  assert.deepEqual(redirects, [])
  assert.equal(useAuthStore.getState().auth.user, null)
})
