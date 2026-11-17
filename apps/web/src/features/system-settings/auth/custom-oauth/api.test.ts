/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { AxiosError } from 'axios'

import { api } from '@/lib/api'

import {
  createCustomOAuthProvider,
  deleteCustomOAuthProvider,
  getCustomOAuthProviders,
  updateCustomOAuthProvider,
} from './api'
import type { CustomOAuthFormValues } from './types'

const provider = {
  id: '9007199254740993',
  revision: '9007199254740995',
  secret_configured: true,
  name: 'Company login',
  slug: 'company',
  icon: '',
  enabled: true,
  client_id: 'company-client',
  client_secret: '',
  authorization_endpoint: 'https://identity.example.test/authorize',
  token_endpoint: 'https://identity.example.test/token',
  user_info_endpoint: 'https://identity.example.test/userinfo',
  scopes: 'profile email',
  user_id_field: 'sub',
  username_field: 'name',
  display_name_field: 'name',
  email_field: 'email',
  well_known: '',
  auth_style: 2,
  pkce_s256: true,
  access_policy: '',
  access_denied_message: '',
}

test('native provider mutations carry exact revisions and reusable operation keys', async () => {
  const original = api.defaults.adapter
  const calls: Array<{ method: string; url: string; body: unknown }> = []
  api.defaults.adapter = async (config) => {
    if (config.method !== 'get') assert.equal(config.skipAuthRefresh, true)
    calls.push({
      method: config.method ?? '',
      url: config.url ?? '',
      body: config.data ? JSON.parse(config.data) : undefined,
    })
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: {
        success: true,
        data: config.method === 'get' ? [provider] : provider,
      },
    }
  }
  try {
    const listed = await getCustomOAuthProviders()
    assert.equal(listed.data?.[0].id, provider.id)
    assert.equal(listed.data?.[0].revision, provider.revision)
    assert.equal(listed.data?.[0].client_secret, '')
    assert.equal(listed.data?.[0].pkce_s256, true)

    const operationKey = 'oauth-operation-synthetic-123'
    const changes = { enabled: false, client_secret: '' }
    await updateCustomOAuthProvider(provider, changes, operationKey)
    await updateCustomOAuthProvider(provider, changes, operationKey)
    assert.deepEqual(calls[1], {
      method: 'put',
      url: `/api/custom-oauth-provider/${provider.id}`,
      body: {
        ...changes,
        expected_revision: provider.revision,
        operation_key: operationKey,
      },
    })
    assert.deepEqual(calls[2], calls[1])

    await deleteCustomOAuthProvider(provider, 'oauth-delete-synthetic-123')
    assert.deepEqual(calls[3], {
      method: 'delete',
      url: `/api/custom-oauth-provider/${provider.id}`,
      body: {
        expected_revision: provider.revision,
        operation_key: 'oauth-delete-synthetic-123',
      },
    })

    const {
      id: _id,
      revision: _revision,
      secret_configured: _secret,
      ...values
    } = provider
    const draft: CustomOAuthFormValues = {
      ...values,
      client_secret: 'synthetic-new-secret',
    }
    await createCustomOAuthProvider(draft, 'oauth-create-synthetic-123')
    assert.deepEqual(calls[4], {
      method: 'post',
      url: '/api/custom-oauth-provider/',
      body: { ...draft, operation_key: 'oauth-create-synthetic-123' },
    })
  } finally {
    api.defaults.adapter = original
  }
})

test('a rejected native revision is not retried through an unversioned write', async () => {
  const original = api.defaults.adapter
  let calls = 0
  api.defaults.adapter = async () => {
    calls++
    throw new Error('409 stale provider revision')
  }
  try {
    await assert.rejects(
      updateCustomOAuthProvider(
        provider,
        { enabled: false },
        'oauth-conflict-synthetic-123'
      ),
      /409 stale provider revision/
    )
    assert.equal(calls, 1)
  } finally {
    api.defaults.adapter = original
  }
})

test('read-only provider metadata is never submitted as configuration', async () => {
  const original = api.defaults.adapter
  let body: Record<string, unknown> = {}
  api.defaults.adapter = async (config) => {
    body = JSON.parse(config.data)
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: { success: true, data: provider },
    }
  }
  try {
    await updateCustomOAuthProvider(
      provider,
      provider,
      'oauth-fields-synthetic-123'
    )
    assert.equal(body.expected_revision, provider.revision)
    assert.equal(body.pkce_s256, true)
    assert.equal('id' in body, false)
    assert.equal('revision' in body, false)
    assert.equal('secret_configured' in body, false)
  } finally {
    api.defaults.adapter = original
  }
})

test('business failures and malformed identities cannot become an editable empty list', async () => {
  const original = api.defaults.adapter
  try {
    for (const payload of [
      { success: false, message: 'settings unavailable', data: [] },
      { success: true, data: [{ ...provider, id: 9 }] },
      { success: true, data: [{ ...provider, revision: '01' }] },
    ]) {
      api.defaults.adapter = async (config) => ({
        config,
        headers: {},
        status: 200,
        statusText: 'OK',
        data: payload,
      })
      await assert.rejects(getCustomOAuthProviders())
    }
  } finally {
    api.defaults.adapter = original
  }
})

test('an expired Session rejects a write without refreshing and replaying it', async () => {
  const original = api.defaults.adapter
  let calls = 0
  api.defaults.adapter = async (config) => {
    calls++
    throw new AxiosError(
      'expired Session',
      'ERR_BAD_REQUEST',
      config,
      undefined,
      {
        config,
        headers: {},
        status: 401,
        statusText: 'Unauthorized',
        data: { success: false },
      }
    )
  }
  try {
    await assert.rejects(
      updateCustomOAuthProvider(
        provider,
        { enabled: false },
        'oauth-expired-synthetic-123'
      ),
      /expired Session/
    )
    assert.equal(calls, 1)
  } finally {
    api.defaults.adapter = original
  }
})
