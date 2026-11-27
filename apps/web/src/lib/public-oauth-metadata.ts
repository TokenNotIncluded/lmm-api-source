/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later. */
import { z } from 'zod'

export const OAUTH_DISCOVERY_PATH = '/.well-known/oauth-authorization-server'
const resourcePath = '/.well-known/oauth-protected-resource/api/oauth2'
const discovery = z.object({
  issuer: z.string(),
  authorization_endpoint: z.string(),
  lmm_client_ids_supported: z.array(z.string()),
  scopes_supported: z.array(z.string()),
  userinfo_endpoint: z.string().optional(),
  jwks_uri: z.string().optional(),
  lmm_isolated_loopback_test: z.boolean().optional(),
})
const resource = z.object({
  resource: z.string(),
  authorization_servers: z.array(z.string()),
})

// Read the configured issuer from this site's own public metadata. Never follow
// a claimed issuer, API base URL, or redirect to retrieve these documents.
export async function readPublicOAuthMetadata(signal: AbortSignal) {
  try {
    const read = async (path: string) => {
      const response = await fetch(path, {
        signal,
        credentials: 'omit',
        mode: 'same-origin',
        redirect: 'error',
        headers: { Accept: 'application/json' },
      })
      if (!response.ok) throw new Error('OAuth metadata unavailable')
      return response.json()
    }
    const [authorization, protectedResource] = await Promise.all([
      read(OAUTH_DISCOVERY_PATH).then((value) => discovery.parse(value)),
      read(resourcePath).then((value) => resource.parse(value)),
    ])
    const { issuer } = authorization
    const url = new URL(issuer)
    const isolated =
      authorization.lmm_isolated_loopback_test === true &&
      url.protocol === 'http:' &&
      ['127.0.0.1', '[::1]'].includes(url.hostname) &&
      typeof window !== 'undefined' &&
      window.location.origin === issuer
    if (
      issuer !== url.origin ||
      (url.protocol !== 'https:' && !isolated) ||
      !protectedResource.authorization_servers.includes(issuer) ||
      protectedResource.resource !== `${issuer}/api/oauth2` ||
      authorization.authorization_endpoint !==
        `${protectedResource.resource}/authorize`
    ) {
      return null
    }
    return {
      issuer,
      pi: authorization.lmm_client_ids_supported.includes('lmm-pi'),
      oidc:
        authorization.scopes_supported.includes('openid') &&
        Boolean(authorization.userinfo_endpoint && authorization.jwks_uri),
    }
  } catch {
    signal.throwIfAborted()
    return null
  }
}
