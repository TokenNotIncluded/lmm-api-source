/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

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
  access_policy: '',
  access_denied_message: '',
}

test('native provider mutations carry exact revisions and reusable operation keys', async () => {
  const original = api.defaults.adapter
  const calls: Array<{ method: string; url: string; body: unknown }> = []
  api.defaults.adapter = async (config) => {
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
