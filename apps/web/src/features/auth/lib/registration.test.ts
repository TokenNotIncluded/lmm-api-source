/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.
*/
import assert from 'node:assert/strict'
import { describe, test } from 'node:test'

import type { FederatedProvider } from './federated-oauth'
import {
  getDisabledOAuthRegistrationMethods,
  canOfferRegistration,
  hasOAuthLoginProvider,
  hasOAuthRegistrationProvider,
  hasRegistrationMethod,
  isPasswordRegistrationEnabled,
  isRegistrationEnabled,
  isOAuthRegistrationAllowed,
} from './registration'

describe('registration availability', () => {
  test('login-only Telegram cannot create a misleading registration entry', () => {
    const status = {
      register_enabled: true,
      oauth_register_enabled: true,
      password_register_enabled: false,
      telegram_oauth: true,
      telegram_bot_name: 'synthetic_bot',
      telegram_register_enabled: false,
    }
    assert.equal(hasOAuthLoginProvider(status), true)
    assert.equal(hasOAuthRegistrationProvider(status), false)
    assert.equal(hasRegistrationMethod(status), false)
    assert.equal(isOAuthRegistrationAllowed(status, 'telegram'), false)
  })
  test('native providers require a server-confirmed registration capability without hiding login', () => {
    const provider: FederatedProvider = {
      id: '41',
      kind: 'oidc',
      slug: 'company',
      name: 'Company',
      icon: '',
      revision: '1',
      native_federated: true,
      qr_code_url: '',
      client_id: 'client',
      authorization_endpoint: 'https://idp.example.test/authorize',
      scopes: 'openid',
      registration_enabled: true,
    }
    const status = {
      native_federated_auth: true,
      password_register_enabled: false,
      federated_oauth_providers: [provider],
    }
    assert.equal(hasOAuthRegistrationProvider(status), true)
    for (const policy of [
      { register_enabled: false },
      { oauth_register_enabled: false },
      { oauth_registration_disabled_methods: ['oidc'] },
      { oauth_registration_disabled_methods: ['federated:41'] },
      {
        federated_oauth_providers: [
          { ...provider, registration_enabled: false },
        ],
      },
      {
        federated_oauth_providers: [
          { ...provider, registration_enabled: undefined },
        ],
      },
    ]) {
      const restricted = { ...status, ...policy }
      assert.equal(
        hasOAuthRegistrationProvider(restricted),
        false,
        JSON.stringify(policy)
      )
      assert.equal(
        hasRegistrationMethod(restricted),
        false,
        JSON.stringify(policy)
      )
      assert.equal(
        hasOAuthLoginProvider(restricted),
        true,
        JSON.stringify(policy)
      )
    }
    assert.equal(isOAuthRegistrationAllowed(status, provider), true)
    const custom = { ...provider, kind: 'custom_oauth' as const }
    const bySlug = {
      ...status,
      oauth_registration_disabled_methods: ['custom:company'],
    }
    assert.equal(isOAuthRegistrationAllowed(bySlug, provider), true)
    assert.equal(isOAuthRegistrationAllowed(bySlug, custom), false)
  })

  test('native federation does not advertise replaced legacy WeChat or OIDC entry points', () => {
    const status = {
      native_federated_auth: true,
      wechat_login: true,
      oidc_enabled: true,
      oidc_client_id: 'client',
      oidc_authorization_endpoint: 'https://sso.example.test/authorize',
      password_register_enabled: false,
    }
    assert.equal(hasOAuthLoginProvider(status), false)
    assert.equal(hasRegistrationMethod(status), false)
    assert.equal(
      hasOAuthLoginProvider({ ...status, native_federated_auth: false }),
      false
    )
  })
  test('advertises a configured Google-only preset without requiring custom records', () => {
    const status = {
      google_oauth: true,
      google_client_id: 'client.apps.googleusercontent.com',
      password_register_enabled: false,
    }
    assert.equal(hasOAuthLoginProvider(status), true)
    assert.equal(hasOAuthRegistrationProvider(status), true)
    assert.equal(hasRegistrationMethod(status), true)
    for (const google of [
      { google_oauth: true },
      { google_oauth: false, google_client_id: 'client' },
      { google_oauth: true, google_client_id: ' ' },
    ]) {
      assert.equal(hasOAuthLoginProvider(google), false)
      assert.equal(hasOAuthRegistrationProvider(google), false)
    }
  })

  test('Google registration switches never disable existing Google login', () => {
    const google = { google_oauth: true, google_client_id: 'client' }
    for (const policy of [
      { register_enabled: false },
      { oauth_register_enabled: false },
      { oauth_registration_disabled_methods: ['GOOGLE'] },
    ]) {
      const status = { ...google, ...policy }
      assert.equal(hasOAuthRegistrationProvider(status), false)
      assert.equal(hasOAuthLoginProvider(status), true)
    }
  })

  test('requires the global registration switch', () => {
    const status = {
      register_enabled: false,
      password_register_enabled: true,
      github_oauth: true,
    }

    assert.equal(isRegistrationEnabled(status), false)
    assert.equal(isPasswordRegistrationEnabled(status), false)
    assert.equal(hasOAuthRegistrationProvider(status), false)
    assert.equal(hasRegistrationMethod(status), false)
  })

  test('hides password registration without hiding OAuth registration', () => {
    const status = {
      register_enabled: true,
      password_register_enabled: false,
      github_oauth: true,
      github_client_id: 'client',
      external_oauth_browser: true,
    }

    assert.equal(isPasswordRegistrationEnabled(status), false)
    assert.equal(hasOAuthRegistrationProvider(status), true)
    assert.equal(hasRegistrationMethod(status), true)
  })

  test('filters disabled OAuth channels, including native custom providers', () => {
    const status = {
      github_oauth: true,
      native_federated_auth: true,
      federated_oauth_providers: [
        {
          id: '1',
          kind: 'custom_oauth' as const,
          revision: '1',
          native_federated: true as const,
          qr_code_url: '',
          name: 'Company SSO',
          slug: 'company-sso',
          icon: '',
          client_id: 'client',
          authorization_endpoint: 'https://sso.example.test/authorize',
          scopes: '',
        },
      ],
      oauth_registration_disabled_methods: ['GITHUB', 'custom:company-sso'],
    }

    assert.deepEqual([...getDisabledOAuthRegistrationMethods(status)].sort(), [
      'custom:company-sso',
      'github',
    ])
    assert.equal(hasOAuthRegistrationProvider(status), false)
  })

  test('does not block existing login when OAuth registration is disabled', () => {
    const status = {
      github_oauth: true,
      github_client_id: 'client',
      external_oauth_browser: true,
      oauth_registration_disabled_methods: ['github'],
    }

    // The helper only answers the sign-up question. Sign-in renders its own
    // OAuthProviders instance without registrationOnly, so this setting does
    // not affect the login path.
    assert.equal(hasOAuthRegistrationProvider(status), false)
    assert.equal(isRegistrationEnabled(status), true)
  })

  test('does not advertise OAuth flags that cannot launch a provider', () => {
    const status = {
      github_oauth: true,
      discord_oauth: true,
      oidc_enabled: true,
      linuxdo_oauth: true,
      telegram_oauth: true,
      custom_oauth_providers: [
        {
          id: 1,
          name: 'Company SSO',
          slug: 'company-sso',
          icon: '',
          client_id: '',
          authorization_endpoint: '',
          scopes: '',
        },
      ],
    }

    assert.equal(hasOAuthLoginProvider(status), false)
    assert.equal(hasOAuthRegistrationProvider(status), false)
  })

  test('keeps login available while registration policy filters a configured provider', () => {
    const status = {
      github_oauth: true,
      github_client_id: 'client',
      external_oauth_browser: true,
      oauth_registration_disabled_methods: ['github'],
    }

    assert.equal(hasOAuthLoginProvider(status), true)
    assert.equal(hasOAuthRegistrationProvider(status), false)
  })

  test('does not advertise sign-up when every registration method is disabled', () => {
    const status = {
      register_enabled: true,
      password_register_enabled: false,
      oauth_register_enabled: false,
      github_oauth: true,
      github_client_id: 'client',
      external_oauth_browser: true,
    }

    assert.equal(hasRegistrationMethod(status), false)
  })

  test('hides OAuth sign-up without hiding OAuth login', () => {
    const status = {
      register_enabled: true,
      oauth_register_enabled: false,
      github_oauth: true,
      github_client_id: 'client',
      external_oauth_browser: true,
    }

    assert.equal(hasOAuthRegistrationProvider(status), false)
    assert.equal(hasOAuthLoginProvider(status), true)
  })

  test('fails closed until live registration capabilities are confirmed', () => {
    const status = { register_enabled: true, password_register_enabled: true }

    assert.equal(canOfferRegistration(status, false), false)
    assert.equal(canOfferRegistration(status, true), true)
    assert.equal(canOfferRegistration(status, false, true), true)
  })
})

test('old OIDC/custom configuration cannot advertise registration or login', () => {
  const status = {
    password_register_enabled: false,
    oidc_enabled: true,
    oidc_client_id: 'old',
    oidc_authorization_endpoint: 'https://sso.example.test/authorize',
    custom_oauth_providers: [
      {
        id: 1,
        name: 'Old SSO',
        slug: 'old-sso',
        icon: '',
        scopes: '',
        client_id: 'old',
        authorization_endpoint: 'https://sso.example.test/authorize',
      },
    ],
  }
  assert.equal(hasOAuthLoginProvider(status), false)
  assert.equal(hasOAuthRegistrationProvider(status), false)
  assert.equal(hasRegistrationMethod(status), false)
})
