/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later. */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { readPublicOAuthMetadata } from './public-oauth-metadata'

test('same-origin metadata supplies the configured issuer and exact Pi client', async () => {
  const saved = globalThis.fetch
  const issuer = 'https://accounts.example.test'
  const resource = issuer + '/api/oauth2'
  let pi = false
  let authority = issuer
  const paths = new Set<string>()
  globalThis.fetch = (async (path, init) => {
    assert.equal(init?.credentials, 'omit')
    assert.equal(init?.redirect, 'error')
    assert.equal(init?.mode, 'same-origin')
    paths.add(String(path))
    return Response.json(
      String(path).includes('protected-resource')
        ? { resource, authorization_servers: [authority] }
        : {
            issuer,
            authorization_endpoint: resource + '/authorize',
            lmm_client_ids_supported: pi ? ['lmm', 'lmm-pi'] : ['lmm'],
            scopes_supported: ['catalog:read'],
          }
    )
  }) as typeof fetch
  try {
    const signal = new AbortController().signal
    assert.deepEqual(await readPublicOAuthMetadata(signal), {
      issuer,
      pi: false,
      oidc: false,
    })
    pi = true
    assert.deepEqual(await readPublicOAuthMetadata(signal), {
      issuer,
      pi: true,
      oidc: false,
    })
    authority = 'https://another.example.test'
    assert.equal(await readPublicOAuthMetadata(signal), null)
    assert.deepEqual([...paths].sort(), [
      '/.well-known/oauth-authorization-server',
      '/.well-known/oauth-protected-resource/api/oauth2',
    ])
  } finally {
    globalThis.fetch = saved
  }
})
