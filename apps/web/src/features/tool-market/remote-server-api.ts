/*
Copyright (C) 2026 LIghtJUNction
SPDX-License-Identifier: AGPL-3.0-or-later
*/
import { z } from 'zod'

import { api } from '@/lib/api'
import type { ApiRequestConfig } from '@/lib/http-client'

const runtime = '/api/tool-market/runtime'
const id = z.string().min(1).max(80)
const installation = z.object({
  id,
  endpoint: z.string(),
  auth: z.enum(['none', 'api_key', 'oauth']),
  enabled: z.boolean(),
  release: z.string(),
})
export type RemoteInstallation = z.infer<typeof installation>
export type RemoteRegistration = {
  endpoint: string
  auth: RemoteInstallation['auth']
  header?: string
  client_id?: string
  scopes?: string
}
export type OwnerScope = NonNullable<ApiRequestConfig['authScope']>

export function remoteServerAPI(authScope: OwnerScope) {
  const options = { authScope, skipErrorHandler: true, skipBusinessError: true }
  const post = async (path: string, body: unknown) =>
    (await api.post<unknown>(`${runtime}/${path}`, body, options)).data
  return {
    list: async (offset: number) =>
      z
        .object({ items: z.array(installation), has_more: z.boolean() })
        .parse(
          await post('installations/list', { query: '', offset, limit: 20 })
        ),
    register: async (input: RemoteRegistration) =>
      z
        .object({ installation: z.object({ id }) })
        .parse(await post('servers/register', input)).installation.id,
    key: async (installation: string, key: string) => {
      const result = await post('credentials/key', { installation, key })
      z.object({ ok: z.literal(true) }).parse(result)
    },
    start: async (installation: string) =>
      z
        .object({ authorization_url: z.string(), state: id })
        .parse(await post('oauth/start', { installation })),
    finish: async (params: URLSearchParams) => {
      const response = await api.get<unknown>(`${runtime}/oauth/callback`, {
        ...options,
        params,
        skipAuthRefresh: true,
        disableDuplicate: true,
      })
      z.object({ authorized: z.literal(true) }).parse(response.data)
    },
    discover: async (installation: string) =>
      z
        .object({ count: z.number().int().nonnegative(), server: id })
        .parse(await post('servers/discover', { installation })).count,
  }
}

export function remoteOAuthRedirectURI(uri: string | undefined) {
  if (uri === `${window.location.origin}${runtime}/oauth/callback`) return uri
}

const marker = '#mcp-oauth-callback&'
const pendingKey = 'tool-market.pending-remote-oauth'

export function readRemoteOAuthCallback() {
  if (
    typeof window === 'undefined' ||
    !window.location.hash.startsWith(marker)
  ) {
    return null
  }
  return new URLSearchParams(window.location.hash.slice(marker.length))
}

export function clearRemoteOAuthFragment() {
  if (window.location.hash.startsWith(marker)) {
    window.history.replaceState(
      window.history.state,
      '',
      window.location.pathname + window.location.search
    )
  }
}

async function stateDigest(state: string) {
  const bytes = await crypto.subtle.digest(
    'SHA-256',
    new TextEncoder().encode(state)
  )
  return Array.from(new Uint8Array(bytes), (byte) =>
    byte.toString(16).padStart(2, '0')
  ).join('')
}

export async function prepareRemoteAuthorization(
  owner: OwnerScope,
  installation: string,
  start: { authorization_url: string; state: string }
) {
  const url = new URL(start.authorization_url)
  if (
    url.protocol !== 'https:' ||
    url.username ||
    url.password ||
    url.hash ||
    url.searchParams.getAll('state').length !== 1 ||
    url.searchParams.get('state') !== start.state
  ) {
    throw new Error('Invalid authorization response')
  }
  const digest = await stateDigest(start.state)
  window.sessionStorage.setItem(
    pendingKey,
    JSON.stringify({
      digest,
      installation,
      owner: owner.userId,
      session: owner.sessionId,
      expires: Date.now() + 300_000,
    })
  )
  return url.href
}

export async function takePendingInstallation(
  owner: OwnerScope,
  params: URLSearchParams
) {
  for (const key of params.keys()) {
    if (
      !['state', 'code', 'iss', 'error', 'error_description'].includes(key) ||
      params.getAll(key).length !== 1
    ) {
      throw new Error('Invalid authorization callback')
    }
  }
  const state = params.get('state')
  if (
    !state ||
    state.length > 80 ||
    Boolean(params.get('code')) === Boolean(params.get('error'))
  ) {
    throw new Error('Invalid authorization callback')
  }
  const pending = JSON.parse(
    window.sessionStorage.getItem(pendingKey) ?? 'null'
  )
  if (
    !pending ||
    pending.owner !== owner.userId ||
    pending.session !== owner.sessionId ||
    pending.expires < Date.now() ||
    pending.digest !== (await stateDigest(state))
  ) {
    throw new Error('No matching authorization')
  }
  window.sessionStorage.removeItem(pendingKey)
  return id.parse(pending.installation)
}
