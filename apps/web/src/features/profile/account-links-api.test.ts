/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import { api } from '@/lib/api'
import { useAuthStore, type AuthBundle } from '@/stores/auth-store'

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
