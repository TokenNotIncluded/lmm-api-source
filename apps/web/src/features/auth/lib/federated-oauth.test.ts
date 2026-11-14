/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import {
  consumeFederatedCeremony,
  consumeFederatedCeremonyForProvider,
  federatedProviders,
  readFederatedCeremony,
  saveFederatedCeremony,
  validateFederatedStart,
  type FederatedProvider,
} from './federated-oauth'

const provider: FederatedProvider = {
  id: '9223372036854775806',
  kind: 'oidc',
  slug: 'controlled-login',
  name: 'Controlled IdP',
  icon: '',
  revision: '1',
  client_id: 'synthetic-client',
  authorization_endpoint: 'https://idp.example.test/authorize',
  scopes: 'openid profile',
  qr_code_url: '',
  native_federated: true,
}
const state = `lmmfe_${'a'.repeat(64)}`
const origin = 'https://web.example.test'
const flow = {
  state,
  expires_at: Math.floor(Date.now() / 1000) + 300,
  authorization_url: `${provider.authorization_endpoint}?tenant=synthetic&scope=openid+profile&client_id=synthetic-client&redirect_uri=${encodeURIComponent(`${origin}/oauth/${provider.slug}`)}&response_type=code&state=${state}&nonce=synthetic-nonce&code_challenge=${'a'.repeat(43)}&code_challenge_method=S256`,
}
function storage() {
  const data = new Map<string, string>()
  return {
    getItem: (key: string) => data.get(key) ?? null,
    setItem: (key: string, value: string) => {
      data.set(key, value)
    },
    removeItem: (key: string) => {
      data.delete(key)
    },
  }
}
test('native catalog keeps string IDs, configured slugs and each protocol kind without accepting unknown providers', () => {
  const catalog = [
    provider,
    { ...provider, id: '2', slug: 'wechat-fixture', kind: 'wechat' },
    { ...provider, id: '3', slug: 'custom-fixture', kind: 'custom_oauth' },
  ]
  assert.equal(
    federatedProviders({
      native_federated_auth: true,
      federated_oauth_providers: catalog as FederatedProvider[],
    }).length,
    3
  )
  for (const bad of [
    { ...provider, kind: 'github' },
    { ...provider, id: 123 },
    { ...provider, untrusted: true },
  ]) {
    assert.throws(() =>
      federatedProviders({
        native_federated_auth: true,
        federated_oauth_providers: [bad] as FederatedProvider[],
      })
    )
  }
  assert.throws(() =>
    federatedProviders({
      native_federated_auth: true,
      federated_oauth_providers: [provider, provider],
    })
  )
})
test('OIDC preserves the exact Core URL and rejects altered endpoint, callback, state, nonce and PKCE', () => {
  assert.equal(
    validateFederatedStart(provider, flow, origin).authorization_url,
    flow.authorization_url
  )
  for (const [key, value] of [
    ['client_id', 'other'],
    ['redirect_uri', `${origin}/oauth/oidc`],
    ['state', `lmmo_${'b'.repeat(64)}`],
    ['code_challenge_method', 'plain'],
    ['code_challenge', 'short'],
    ['nonce', ''],
  ]) {
    const url = new URL(flow.authorization_url)
    url.searchParams.set(key, value)
    assert.throws(() =>
      validateFederatedStart(
        provider,
        { ...flow, authorization_url: url.toString() },
        origin
      )
    )
  }
  for (const value of [
    `${flow.authorization_url}&state=${state}`,
    flow.authorization_url.replace('https:', 'http:'),
    flow.authorization_url.replace('idp.example.test', 'evil.example.test'),
    `${flow.authorization_url}#fragment`,
    flow.authorization_url.replace('https://', 'https://user@'),
  ]) {
    assert.throws(() =>
      validateFederatedStart(
        provider,
        { ...flow, authorization_url: value },
        origin
      )
    )
  }
  assert.throws(() =>
    validateFederatedStart(provider, { ...flow, expires_at: 1 }, origin)
  )
})
test('custom OAuth does not require an OIDC nonce/id_token or invent PKCE; WeChat only accepts the empty authorization URL', () => {
  const url = new URL(flow.authorization_url)
  for (const key of ['nonce', 'code_challenge', 'code_challenge_method']) {
    url.searchParams.delete(key)
  }
  const coreWithoutPkce = new URL(flow.authorization_url)
  coreWithoutPkce.searchParams.delete('code_challenge')
  coreWithoutPkce.searchParams.delete('code_challenge_method')
  assert.equal(
    validateFederatedStart(
      provider,
      { ...flow, authorization_url: coreWithoutPkce.toString() },
      origin
    ).authorization_url,
    coreWithoutPkce.toString()
  )
  assert.equal(
    validateFederatedStart(
      { ...provider, kind: 'custom_oauth' },
      { ...flow, authorization_url: url.toString() },
      origin
    ).state,
    state
  )
  assert.equal(
    validateFederatedStart(
      { ...provider, kind: 'wechat' },
      { ...flow, authorization_url: '' },
      origin
    ).state,
    state
  )
  assert.throws(() =>
    validateFederatedStart({ ...provider, kind: 'wechat' }, flow, origin)
  )
})
test('ceremonies bind ID, slug, intent, exact state and expiry and can be consumed only once', () => {
  const store = storage()
  saveFederatedCeremony(store, provider, flow, 'login')
  assert.equal(
    readFederatedCeremony(store, state, provider.slug, 'login').providerId,
    provider.id
  )
  for (const [slug, intent] of [
    ['oidc', 'login'],
    [provider.slug, 'bind'],
  ] as const) {
    assert.throws(() => consumeFederatedCeremony(store, state, slug, intent))
  }
  assert.equal(
    consumeFederatedCeremony(store, state, provider.slug, 'login').providerId,
    provider.id
  )
  assert.throws(() =>
    consumeFederatedCeremony(store, state, provider.slug, 'login')
  )
  saveFederatedCeremony(store, provider, { ...flow, expires_at: 1 }, 'login')
  assert.throws(() =>
    consumeFederatedCeremony(store, state, provider.slug, 'login')
  )
})

test('provider-bound consumption reads and removes once, validates intent and provider before deleting, and owns slug parsing', () => {
  const base = storage()
  let reads = 0,
    removes = 0
  const store = {
    ...base,
    getItem: (key: string) => {
      reads++
      return base.getItem(key)
    },
    removeItem: (key: string) => {
      removes++
      base.removeItem(key)
    },
  }
  saveFederatedCeremony(base, provider, flow, 'bind')
  assert.throws(() =>
    consumeFederatedCeremonyForProvider(store, state, '1', 'bind')
  )
  assert.throws(() =>
    consumeFederatedCeremonyForProvider(store, state, provider.id, 'login')
  )
  assert.equal(removes, 0)
  reads = 0
  assert.equal(
    consumeFederatedCeremonyForProvider(store, state, provider.id, 'bind').slug,
    provider.slug
  )
  assert.equal(reads, 1)
  assert.equal(removes, 1)
  assert.throws(() =>
    consumeFederatedCeremonyForProvider(store, state, provider.id, 'bind')
  )
})
