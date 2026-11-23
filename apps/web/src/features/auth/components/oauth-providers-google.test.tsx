/*
Copyright (C) 2026 LIghtJUNction

SPDX-License-Identifier: AGPL-3.0-or-later
*/
// @ts-expect-error Bun's test module is available only in the test runtime.
import { mock } from 'bun:test'
import assert from 'node:assert/strict'
import { after, afterEach, describe, test } from 'node:test'

import { Window } from 'happy-dom'

import type { SystemStatus } from '../types'

const dom = new Window()
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'SVGElement',
  'Node',
  'Element',
  'Event',
  'MouseEvent',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: dom[key],
  })
}
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
const clicks: string[] = []
const consent: boolean[] = []
let oauthPending = false
let telegramPending = false
mock.module('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string, options?: { name?: string }) =>
      key.replace('{{name}}', options?.name ?? ''),
  }),
}))
mock.module('../hooks/use-oauth-login', () => ({
  useOAuthLogin: (_: unknown, __: unknown, acceptedLegal: boolean) => {
    consent.push(acceptedLegal)
    return {
      isLoading: oauthPending,
      githubButtonText: '',
      githubButtonDisabled: false,
      handleGoogleLogin: () => clicks.push('builtin:google'),
      handleGitHubLogin: () => clicks.push('github'),
      handleDiscordLogin: () => {},
      handleFederatedLogin: (provider: { id: string; slug: string }) =>
        clicks.push(`federated:${provider.id}:${provider.slug}`),
      handleLinuxDOLogin: () => {},
      handleTelegramLogin: () => {},
      isTelegramDialogOpen: false,
      isTelegramPending: telegramPending,
      handleTelegramAuthorization: () => {},
      handleTelegramDialogChange: () => {},
    }
  },
}))
mock.module('@/assets/brand-icons', () => {
  const icon = () => <span aria-hidden='true' />
  return {
    IconDiscord: icon,
    IconGithub: icon,
    IconGoogle: icon,
    IconLinuxDo: icon,
    IconTelegram: icon,
    IconWeChat: icon,
  }
})
mock.module('./telegram-login-dialog', () => ({
  TelegramLoginDialog: () => null,
}))
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { OAuthProviders } = await import('./oauth-providers')
let root: ReturnType<typeof createRoot> | undefined

const legacy = {
  id: 73,
  name: 'Google',
  slug: 'legacy-google',
  icon: 'google',
  client_id: 'legacy-client',
  authorization_endpoint: 'https://accounts.google.com/o/oauth2/v2/auth',
  scopes: 'openid profile email',
}
const supported = {
  google_oauth: true,
  google_client_id: 'builtin-client',
  custom_oauth_providers: [legacy],
}
async function render(
  status: SystemStatus | null,
  props: {
    registrationOnly?: boolean
    disabled?: boolean
    isWeChatLoading?: boolean
    acceptedLegal?: boolean
    registrationCapabilitiesReady?: boolean
  } = {}
) {
  const host = document.createElement('div')
  document.body.append(host)
  const currentRoot = createRoot(host)
  root = currentRoot
  await act(async () =>
    currentRoot.render(
      <OAuthProviders
        status={status}
        registrationCapabilitiesReady
        featureGoogle
        onWeChatLogin={(provider) =>
          clicks.push(`wechat:${provider?.id}:${provider?.slug}`)
        }
        {...props}
      />
    )
  )
  return host
}
async function unmount() {
  const currentRoot = root
  if (currentRoot) await act(async () => currentRoot.unmount())
  root = undefined
}
function getButton(host: HTMLElement) {
  const button = host.querySelector<HTMLButtonElement>('button')
  assert.ok(button)
  return button
}
afterEach(async () => {
  await unmount()
  document.body.replaceChildren()
  clicks.length = 0
  oauthPending = false
  telegramPending = false
  consent.length = 0
})
after(() => dom.close())

test('browser OAuth pending is announced and clears when the attempt settles', async () => {
  oauthPending = true
  const host = await render(supported, { acceptedLegal: true })
  const button = getButton(host)
  assert.equal(button.disabled, true)
  assert.equal(button.getAttribute('aria-busy'), 'true')
  assert.equal(
    host.querySelector('[role="status"]')?.textContent,
    'Signing in...'
  )
  assert.equal(
    host.querySelector('[role="status"]')?.getAttribute('aria-live'),
    'polite'
  )
  await act(async () => button.click())
  assert.deepEqual(clicks, [])
  oauthPending = false
  await act(async () =>
    root?.render(
      <OAuthProviders
        status={supported}
        registrationCapabilitiesReady
        featureGoogle
        acceptedLegal
      />
    )
  )
  assert.equal(getButton(host).disabled, false)
  assert.equal(getButton(host).getAttribute('aria-busy'), 'false')
  assert.equal(host.querySelector('[role="status"]'), null)
  await act(async () => getButton(host).click())
  assert.deepEqual(clicks, ['builtin:google'])
})

for (const provider of ['Telegram', 'WeChat'] as const) {
  test(`${provider} pending has visible accessible progress`, async () => {
    telegramPending = provider === 'Telegram'
    const host = await render(supported, {
      isWeChatLoading: provider === 'WeChat',
    })
    assert.equal(
      host.querySelector('[role="status"]')?.textContent,
      'Signing in...'
    )
    assert.equal(
      host.querySelector('[role="status"]')?.getAttribute('aria-live'),
      'polite'
    )
  })
}

describe('Google provider buttons', () => {
  test('native supported browser Google wins, custom duplicate is not invoked', async () => {
    const host = await render(supported, { acceptedLegal: true })
    const buttons = host.querySelectorAll<HTMLButtonElement>('button')
    assert.equal(buttons.length, 1)
    assert.equal(buttons[0].getAttribute('aria-label'), 'Continue with Google')
    await act(async () => buttons[0].click())
    assert.deepEqual(clicks, ['builtin:google'])
    assert.equal(consent.at(-1), true)
  })

  test('unsupported/disabled/unconfigured/missing-flag native does not show old custom buttons', async () => {
    for (const status of [
      { custom_oauth_providers: [legacy] },
      { ...supported, google_oauth: false },
      { ...supported, google_client_id: '' },
      { ...supported, google_oauth: undefined },
    ]) {
      const host = await render(status)
      assert.equal(host.querySelector('button'), null)
      assert.deepEqual(clicks, [])
      await unmount()
    }
  })

  test('multiple old custom Google records render no buttons', async () => {
    const host = await render({
      custom_oauth_providers: [legacy, { ...legacy, id: 74, slug: 'google' }],
    })
    assert.equal(host.querySelector('button'), null)
    assert.deepEqual(clicks, [])
  })

  test('builtin registration disable cannot revive legacy registration', async () => {
    const host = await render(
      { ...supported, oauth_registration_disabled_methods: ['google'] },
      { registrationOnly: true }
    )
    assert.equal(host.querySelector('button'), null)
    assert.deepEqual(clicks, [])
  })

  test('registration method restrictions do not hide existing sign-in', async () => {
    const host = await render({
      ...supported,
      oauth_registration_disabled_methods: ['google', 'custom:legacy-google'],
    })
    const button = getButton(host)
    assert.match(button.textContent ?? '', /Sign in only/)
    assert.ok(
      document.getElementById(button.getAttribute('aria-describedby') ?? '')
    )
    await act(async () => button.click())
    assert.deepEqual(clicks, ['builtin:google'])
  })

  test('disabled consent gate remains disabled and does not invoke Google', async () => {
    const host = await render(supported, {
      disabled: true,
      acceptedLegal: false,
    })
    const button = getButton(host)
    assert.equal(button.disabled, true)
    await act(async () => button.click())
    assert.deepEqual(clicks, [])
    assert.equal(consent.at(-1), false)
  })

  test('missing flags with no fallback render no misleading native button', async () => {
    const host = await render({ google_client_id: 'native-only-client' })
    assert.equal(host.querySelector('button'), null)
  })
})

test('global registration switches filter sign-up buttons but preserve labelled sign-in', async () => {
  for (const policy of [
    { register_enabled: false },
    { oauth_register_enabled: false },
  ]) {
    const status = { ...supported, ...policy }
    const signup = await render(status, { registrationOnly: true })
    assert.equal(signup.querySelector('button'), null)
    await unmount()
    const login = await render(status)
    assert.match(getButton(login).textContent ?? '', /Sign in only/)
    assert.equal(getButton(login).disabled, false)
    await unmount()
  }
})

test('cached capabilities keep login usable without advertising an unverified registration policy', async () => {
  const login = await render(supported, {
    registrationCapabilitiesReady: false,
  })
  assert.equal(getButton(login).disabled, false)
  assert.doesNotMatch(
    login.textContent ?? '',
    /Sign in or register|Sign in only/
  )
  await unmount()
  const signup = await render(supported, {
    registrationOnly: true,
    registrationCapabilitiesReady: false,
  })
  assert.equal(signup.querySelector('button'), null)
})

test('Telegram remains labelled login-only even when another OAuth provider can register', async () => {
  const status = {
    ...supported,
    telegram_oauth: true,
    telegram_bot_name: 'synthetic_bot',
    telegram_register_enabled: false,
  }
  const login = await render(status)
  const telegram = login.querySelector<HTMLButtonElement>(
    'button[aria-label="Continue with Telegram"]'
  )
  assert.ok(telegram)
  assert.match(telegram.textContent ?? '', /Sign in only/)
  await unmount()
  const signup = await render(status, { registrationOnly: true })
  assert.equal(
    signup.querySelector('button[aria-label="Continue with Telegram"]'),
    null
  )
  assert.equal(signup.querySelectorAll('button').length, 1)
})

test('mixed native provider policies show effective capabilities and filter registration independently', async () => {
  const provider = {
    id: '91',
    kind: 'oidc' as const,
    slug: 'register-sso',
    name: 'Registration SSO',
    icon: '',
    revision: '1',
    client_id: 'synthetic',
    authorization_endpoint: 'https://idp.example.test/authorize',
    scopes: 'openid',
    qr_code_url: '',
    native_federated: true as const,
    registration_enabled: true,
  }
  const loginOnly = {
    ...provider,
    id: '92',
    slug: 'login-sso',
    name: 'Login SSO',
    registration_enabled: false,
  }
  const unknown = {
    ...provider,
    id: '93',
    slug: 'unknown-sso',
    name: 'Unknown SSO',
    registration_enabled: undefined,
  }
  const status = {
    native_federated_auth: true,
    federated_oauth_providers: [provider, loginOnly, unknown],
  }
  const login = await render(status)
  const buttons = [...login.querySelectorAll<HTMLButtonElement>('button')]
  assert.equal(buttons.length, 3)
  assert.match(buttons[0].textContent ?? '', /Sign in or register/)
  assert.match(buttons[1].textContent ?? '', /Sign in only/)
  assert.match(buttons[2].textContent ?? '', /Sign in only/)
  await act(async () => buttons[1].click())
  assert.deepEqual(clicks, ['federated:92:login-sso'])
  await unmount()
  const signup = await render(status, { registrationOnly: true })
  assert.equal(signup.querySelectorAll('button').length, 1)
  assert.equal(
    getButton(signup).getAttribute('aria-label'),
    'Continue with Registration SSO'
  )
})

for (const kind of ['oidc', 'custom_oauth', 'wechat'] as const) {
  test(`native ${kind} catalog buttons retain configured IDs and slugs without legacy cards`, async () => {
    const selected = {
      id: '81',
      kind,
      slug: 'controlled-sso',
      name: 'Synthetic SSO',
      icon: '',
      revision: '1',
      client_id: 'synthetic-client',
      authorization_endpoint: 'https://idp.example.test/authorize',
      scopes: 'openid',
      qr_code_url: '',
      native_federated: true as const,
    }
    const host = await render({
      native_federated_auth: true,
      federated_oauth_providers: [selected],
      oidc_enabled: true,
      oidc_client_id: 'synthetic-client',
      oidc_authorization_endpoint: selected.authorization_endpoint,
      custom_oauth_providers: [{ ...legacy, slug: selected.slug }],
    })
    const buttons = host.querySelectorAll<HTMLButtonElement>('button')
    assert.equal(buttons.length, 1)
    assert.equal(
      buttons[0].getAttribute('aria-label'),
      'Continue with Synthetic SSO'
    )
    await act(async () => buttons[0].click())
    assert.deepEqual(clicks, [
      kind === 'wechat'
        ? 'wechat:81:controlled-sso'
        : 'federated:81:controlled-sso',
    ])
  })
}

test('legacy OIDC and fixed providers without external browser support have no ineffective buttons', async () => {
  const host = await render({
    oidc_enabled: true,
    oidc_client_id: 'old-client',
    oidc_authorization_endpoint: 'https://idp.example.test/authorize',
    github_oauth: true,
    github_client_id: 'github-client',
    discord_oauth: true,
    discord_client_id: 'discord-client',
    linuxdo_oauth: true,
    linuxdo_client_id: 'linuxdo-client',
    external_oauth_browser: false,
  })
  assert.equal(host.querySelector('button'), null)
})

test('external browser providers remain actionable', async () => {
  const host = await render({
    github_oauth: true,
    github_client_id: 'github-client',
    discord_oauth: true,
    discord_client_id: 'discord-client',
    linuxdo_oauth: true,
    linuxdo_client_id: 'linuxdo-client',
    external_oauth_browser: true,
  })
  const buttons = host.querySelectorAll<HTMLButtonElement>('button')
  assert.equal(buttons.length, 3)
  await act(async () => buttons[0].click())
  assert.deepEqual(clicks, ['github'])
})

test('old WeChat flag without native catalog never renders a login button', async () => {
  for (const native of [undefined, false, true]) {
    const host = await render({
      wechat_login: true,
      native_federated_auth: native,
    })
    assert.equal(host.querySelector('button'), null)
    assert.deepEqual(clicks, [])
    await unmount()
  }
})
