/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
// @ts-expect-error Bun's test module is available only in the test runtime.
import { mock } from 'bun:test'
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { Window } from 'happy-dom'

import type { SystemStatus } from '@/features/auth/types'

const dom = new Window({
  url: 'https://console.example.test/system-settings/auth/basic-auth',
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
  'MouseEvent',
  'PointerEvent',
  'MutationObserver',
  'ResizeObserver',
  'getComputedStyle',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: dom[key],
  })
}
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
let status: SystemStatus
const writes: Array<{ key: string; value: string | boolean }> = []
mock.module('@/hooks/use-status', () => ({
  useStatus: () => ({
    status,
    capabilitiesReady: true,
    error: null,
    refetch() {},
  }),
}))
mock.module('../hooks/use-update-option', () => ({
  useUpdateOption: () => ({
    isPending: false,
    mutateAsync: async (update: (typeof writes)[number]) => {
      writes.push(update)
    },
  }),
}))
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { BasicAuthSection } = await import('./basic-auth-section')
const { OAuthRegistrationPolicy } = await import('./oauth-registration-policy')
const i18n = createInstance()
await i18n.use(initReactI18next).init({ lng: 'en', resources: {} })
const provider = {
  id: '81',
  kind: 'custom_oauth' as const,
  slug: 'company',
  name: 'Company SSO',
  icon: '',
  revision: '1',
  native_federated: true as const,
  qr_code_url: '',
  client_id: 'client',
  authorization_endpoint: 'https://idp.example.test/authorize',
  scopes: 'openid',
  registration_enabled: false,
}
const defaults = {
  PasswordLoginEnabled: true,
  PasswordRegisterEnabled: true,
  OAuthRegisterEnabled: true,
  RegistrationDisabledMethods: '',
  EmailVerificationEnabled: false,
  RegisterEnabled: true,
  EmailDomainRestrictionEnabled: false,
  EmailAliasRestrictionEnabled: false,
  EmailDomainWhitelist: '',
}
let root: ReturnType<typeof createRoot> | undefined
async function render(
  patch: Partial<typeof defaults> = {},
  policyOnly = false
) {
  status = {
    native_federated_auth: true,
    federated_oauth_providers: [provider],
    google_oauth: true,
    google_client_id: 'client',
  }
  const host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
  await act(async () =>
    root?.render(
      <I18nextProvider i18n={i18n}>
        {policyOnly ? (
          <OAuthRegistrationPolicy />
        ) : (
          <BasicAuthSection defaultValues={{ ...defaults, ...patch }} />
        )}
      </I18nextProvider>
    )
  )
  return host
}
function channel(host: HTMLElement, name: string) {
  const element = host.querySelector<HTMLButtonElement>(
    `[role="switch"][aria-label="Allow new accounts through ${name}"]`
  )
  assert.ok(element, name)
  return element
}
afterEach(async () => {
  await act(async () => root?.unmount())
  root = undefined
  writes.length = 0
  document.body.replaceChildren()
})
after(() => dom.close())

test('Google and native providers reflect editable policy and preserve unrelated restrictions on save', async () => {
  const host = await render({
    RegistrationDisabledMethods: 'google,custom:company,custom:other',
  })
  assert.equal(channel(host, 'Google').getAttribute('aria-checked'), 'false')
  const native = channel(host, 'Company SSO')
  assert.equal(native.getAttribute('aria-checked'), 'false')
  await act(async () => native.click())
  assert.equal(native.getAttribute('aria-checked'), 'true')
  const form = host.querySelector('form')
  assert.ok(form)
  await act(async () => {
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
    await new Promise((resolve) => setTimeout(resolve, 10))
  })
  assert.deepEqual(writes, [
    { key: 'RegistrationDisabledMethods', value: 'custom:other,google' },
  ])
})

test('global and protocol-wide restrictions cannot be advertised as allowed by individual switches', async () => {
  const host = await render({
    RegistrationDisabledMethods: 'custom_oauth',
    OAuthRegisterEnabled: false,
  })
  for (const name of ['Google', 'Company SSO']) {
    assert.equal(channel(host, name).getAttribute('aria-checked'), 'false')
    assert.equal(channel(host, name).getAttribute('aria-disabled'), 'true')
  }
  assert.match(
    host.textContent ?? '',
    /Registration is disabled by a channel-wide rule/
  )
})

test('native read-only policy shows the server capability rather than editable defaults', async () => {
  const host = await render({}, true)
  const rows = [...host.querySelectorAll('dl > div')]
  assert.equal(rows.length, 2)
  assert.match(rows[0].textContent ?? '', /Google.*Sign in or register/)
  assert.match(rows[1].textContent ?? '', /Company SSO.*Sign in only/)
})
