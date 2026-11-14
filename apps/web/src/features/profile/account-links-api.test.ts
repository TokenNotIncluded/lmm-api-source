/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import { api } from '@/lib/api'
import { useAuthStore, type AuthBundle } from '@/stores/auth-store'
import { installSupportedAuthBrowser } from '@/test-support/supported-auth-browser'

import {
  ACCOUNT_PROVIDER_ISSUERS,
  accountLinkScope,
  beginAccountBinding,
  finishAccountBinding,
  readAccountLinks,
  unlinkAccount,
} from './account-links-api'

const originalAuth = useAuthStore.getState().auth
const adapter = api.defaults.adapter
const flow = `lmmo_${'a'.repeat(64)}`
const origin = 'https://bindings.example.test'
// Normal supported browser, including the API-only cases above the DOM case.
installSupportedAuthBrowser({ url: origin })
function bundle(
  sid = 'synthetic-account-links',
  token = 'synthetic-access'
): AuthBundle {
  return {
    access_token: token,
    token_type: 'Bearer',
    access_expires_at: 2000000000,
    user: { id: 501, username: 'synthetic-user', role: 1 },
    session: {
      sid,
      current: true,
      login_method: 'password',
      ip: '127.0.0.1',
      user_agent: 'synthetic',
      created_at: 1,
      last_active_at: 1,
      expires_at: 2000000000,
    },
  }
}
const link = {
  issuer: ACCOUNT_PROVIDER_ISSUERS.google,
  subject: 'synthetic-subject',
  revision: '0',
  active: true,
}
const listing = {
  contract: 'core.account-links.v1',
  expected_auth_version: '3',
  links: [link],
  providers: Object.entries(ACCOUNT_PROVIDER_ISSUERS).map(
    ([provider, issuer]) => ({ provider, issuer, can_bind: true })
  ),
}
function authorization(state = flow) {
  const url = new URL('https://github.com/login/oauth/authorize')
  for (const [key, value] of Object.entries({
    client_id: 'synthetic-client',
    response_type: 'code',
    scope: 'read:user',
    state,
    redirect_uri: `${origin}/api/oauth/github`,
    code_challenge: 'b'.repeat(43),
    code_challenge_method: 'S256',
  })) {
    url.searchParams.set(key, value)
  }
  return url.toString()
}
afterEach(() => {
  api.defaults.adapter = adapter
  useAuthStore.setState({ auth: originalAuth })
})

test('account unlink keeps exact CAS strings and applies only the same-user, same-family rotation', async () => {
  useAuthStore.getState().auth.setBundle(bundle())
  const scope = accountLinkScope()
  let writes = 0
  api.defaults.adapter = async (config) => {
    assert.deepEqual(config.authScope, scope)
    assert.equal(config.skipAuthRefresh, true)
    if (config.method === 'get') {
      return {
        config,
        headers: {},
        status: 200,
        statusText: 'OK',
        data: { success: true, data: listing },
      }
    }
    writes++
    assert.equal(config.url, '/api/user/account-links/unlink')
    assert.deepEqual(JSON.parse(String(config.data)), {
      issuer: link.issuer,
      subject: link.subject,
      expected_revision: '0',
      expected_auth_version: '3',
    })
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: { success: true, data: bundle(undefined, 'rotated-access') },
    }
  }
  const view = await readAccountLinks(scope)
  await unlinkAccount(scope, view.links[0]!, view.expected_auth_version)
  assert.equal(writes, 1)
  assert.equal(useAuthStore.getState().auth.accessToken, 'rotated-access')
  await assert.rejects(unlinkAccount(scope, { ...link, revision: '1.5' }, '3'))
  await assert.rejects(unlinkAccount(scope, { ...link, active: false }, '3'))
  assert.equal(writes, 1)
})

test('binding preserves the fixed server authorization URL and refuses a mismatched returned state', async () => {
  useAuthStore.getState().auth.setBundle(bundle())
  const scope = accountLinkScope()
  let mismatch = false
  api.defaults.adapter = async (config) => {
    assert.equal(config.url, '/api/user/account-links/github/begin')
    assert.deepEqual(JSON.parse(String(config.data)), {
      expected_auth_version: '3',
    })
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: {
        success: true,
        data: {
          state: flow,
          authorization_url: authorization(
            mismatch ? `lmmo_${'c'.repeat(64)}` : flow
          ),
          expires_at: 2000000000,
        },
      },
    }
  }
  const start = await beginAccountBinding(
    scope,
    'github',
    '3',
    { github_client_id: 'synthetic-client' },
    origin
  )
  assert.equal(start.authorization_url, authorization())
  mismatch = true
  await assert.rejects(
    beginAccountBinding(
      scope,
      'github',
      '3',
      { github_client_id: 'synthetic-client' },
      origin
    )
  )
})

test('a delayed callback cannot replace a newer login and Discord callbacks require issuer proof', async () => {
  useAuthStore.getState().auth.setBundle(bundle())
  const scope = accountLinkScope()
  let writes = 0
  api.defaults.adapter = async (config) => {
    writes++
    useAuthStore
      .getState()
      .auth.setBundle(bundle('replacement-family', 'replacement-access'))
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: { success: true, data: bundle(undefined, 'late-access') },
    }
  }
  await assert.rejects(
    finishAccountBinding(scope, 'google', {
      state: flow,
      code: 'synthetic-code',
    })
  )
  assert.equal(useAuthStore.getState().auth.accessToken, 'replacement-access')
  assert.equal(writes, 1)
  await assert.rejects(
    finishAccountBinding(accountLinkScope(), 'discord', {
      state: flow,
      code: 'synthetic-code',
    })
  )
  await assert.rejects(
    finishAccountBinding(scope, 'google', {
      state: flow,
      code: 'synthetic-code',
    })
  )
  assert.equal(writes, 1)
})

test('dynamic federated descriptors coexist with fixed OAuth and reject mismatched identity metadata', async () => {
  const { accountLinksSchema } = await import('./account-links-api')
  const descriptor = {
    provider: 'federated:73',
    provider_id: '73',
    issuer: 'urn:lmm:federated:73',
    can_bind: true,
    slug: 'synthetic-wechat',
    name: 'WeChat',
    kind: 'wechat',
    icon: '',
    qr_code_url: '',
    native_federated: true,
  }
  const combined = { ...listing, providers: [...listing.providers, descriptor] }
  assert.equal(accountLinksSchema.parse(combined).providers.length, 5)
  const identity = {
    issuer: 'urn:lmm:federated:73',
    provider_id: '73',
    subject: 'synthetic-subject',
    revision: '1',
    active: true,
    available: false,
    slug: descriptor.slug,
    name: descriptor.name,
    kind: descriptor.kind,
  }
  assert.equal(
    accountLinksSchema.parse({ ...combined, links: [identity] }).links.length,
    1
  )
  assert.throws(() =>
    accountLinksSchema.parse({
      ...combined,
      links: [{ ...identity, provider_id: '74' }],
    })
  )
  const { provider_id: _id, ...missingId } = identity
  assert.throws(() =>
    accountLinksSchema.parse({ ...combined, links: [missingId] })
  )
  void _id

  for (const changed of [
    { provider_id: '74' },
    { issuer: 'https://github.com' },
    { native_federated: false },
    { kind: 'github' },
    { provider: 'github' },
  ]) {
    assert.throws(() =>
      accountLinksSchema.parse({
        ...combined,
        providers: [...listing.providers, { ...descriptor, ...changed }],
      })
    )
  }
})
test('native binding uses lmmfe only and rotates the same SID before CAS unlink', async () => {
  const { Window } = await import('happy-dom')
  const previous = Object.getOwnPropertyDescriptor(globalThis, 'window')
  const dom = new Window({ url: origin })
  Object.defineProperty(globalThis, 'window', {
    configurable: true,
    value: dom,
  })
  const native = {
    id: '73',
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
  }
  const started = {
    state: `lmmfe_${'e'.repeat(64)}`,
    expires_at: 2000000000,
    authorization_url: '',
  }
  useAuthStore.getState().auth.setBundle(bundle())
  const scope = accountLinkScope()
  const calls: string[] = []
  api.defaults.adapter = async (config) => {
    calls.push(config.url!)
    assert.deepEqual(config.authScope, scope)
    if (config.url!.endsWith('/begin')) {
      assert.deepEqual(JSON.parse(config.data), { expected_auth_version: '3' })
    }
    if (config.url!.endsWith('/finish')) {
      assert.deepEqual(JSON.parse(config.data), {
        state: started.state,
        code: 'synthetic-code',
      })
    }
    return {
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: {
        success: true,
        data: config.url!.endsWith('/begin')
          ? started
          : bundle(scope.sessionId, 'rotated-native-access'),
      },
    }
  }
  try {
    await beginAccountBinding(
      scope,
      'federated:73',
      '3',
      { native_federated_auth: true, federated_oauth_providers: [native] },
      origin
    )
    await assert.rejects(
      finishAccountBinding(scope, 'github', {
        state: started.state,
        code: 'synthetic-code',
      })
    )
    await assert.rejects(
      finishAccountBinding(scope, 'federated:73', {
        state: flow,
        code: 'synthetic-code',
      })
    )
    await finishAccountBinding(scope, 'federated:73', {
      state: started.state,
      code: 'synthetic-code',
    })
    assert.equal(useAuthStore.getState().auth.session?.sid, scope.sessionId)
    assert.equal(
      useAuthStore.getState().auth.accessToken,
      'rotated-native-access'
    )
    await assert.rejects(
      finishAccountBinding(scope, 'federated:73', {
        state: started.state,
        code: 'synthetic-code',
      })
    )
    await unlinkAccount(
      scope,
      {
        issuer: 'urn:lmm:federated:73',
        provider_id: '73',
        slug: native.slug,
        name: native.name,
        kind: native.kind,
        available: true,
        subject: 'synthetic-subject',
        revision: '0',
        active: true,
      },
      '3'
    )
    assert.deepEqual(calls, [
      '/api/user/account-links/federated:73/begin',
      '/api/user/account-links/federated:73/finish',
      '/api/user/account-links/unlink',
    ])
  } finally {
    dom.close()
    if (previous) Object.defineProperty(globalThis, 'window', previous)
    else Reflect.deleteProperty(globalThis, 'window')
  }
})
