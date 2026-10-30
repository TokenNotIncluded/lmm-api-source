/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

import { validateExternalBrowserAuthorizationUrl } from '@/features/auth/lib/external-oauth'
import { validateGoogleBrowserAuthorizationUrl } from '@/features/auth/lib/google-oauth'
import type { SystemStatus } from '@/features/auth/types'
import { api } from '@/lib/api'
import {
  applyAuthBundle,
  clearAuthentication,
  isAuthBundle,
  withAuthenticatedCookieWrite,
} from '@/lib/auth-session'
import { useAuthStore } from '@/stores/auth-store'

const version = z
  .string()
  .regex(/^[1-9][0-9]{0,18}$/)
  .pipe(z.string().refine((value) => BigInt(value) <= 9223372036854775807n))
const revision = z
  .string()
  .regex(/^(0|[1-9][0-9]{0,18})$/)
  .pipe(z.string().refine((value) => BigInt(value) <= 9223372036854775807n))
const provider = z.enum(['google', 'github', 'linuxdo', 'discord'])
export type AccountProvider = z.infer<typeof provider>
export const ACCOUNT_PROVIDER_ISSUERS: Record<AccountProvider, string> = {
  google: 'https://accounts.google.com',
  github: 'https://github.com',
  linuxdo: 'https://connect.linux.do',
  discord: 'https://discord.com',
}
const linkSchema = z
  .object({
    issuer: z.string().min(1).max(512),
    subject: z.string().min(1).max(512),
    revision,
    active: z.boolean(),
  })
  .strict()
export const accountLinksSchema = z
  .object({
    contract: z.literal('core.account-links.v1'),
    expected_auth_version: version,
    links: z.array(linkSchema).max(101),
    providers: z
      .array(
        z
          .object({ provider, issuer: z.string(), can_bind: z.boolean() })
          .strict()
      )
      .length(4)
      .refine(
        (rows) =>
          new Set(rows.map((row) => row.provider)).size === 4 &&
          rows.every(
            (row) => row.issuer === ACCOUNT_PROVIDER_ISSUERS[row.provider]
          )
      ),
  })
  .strict()
const bindStartSchema = z
  .object({
    authorization_url: z.string().min(1),
    state: z.string().regex(/^lmmo_[a-f0-9]{64}$/),
    expires_at: z.number().int().positive(),
  })
  .strict()
export type AccountLink = z.infer<typeof linkSchema>
export type AccountLinks = z.infer<typeof accountLinksSchema>
export type AccountLinkScope = { userId: number; sessionId: string }

export function accountLinkScope(): AccountLinkScope {
  const auth = useAuthStore.getState().auth
  if (!auth.user || !auth.session) throw new Error('Session expired!')
  return { userId: auth.user.id, sessionId: auth.session.sid }
}
export function assertAccountLinkScope(scope: AccountLinkScope) {
  const auth = useAuthStore.getState().auth
  if (auth.user?.id !== scope.userId || auth.session?.sid !== scope.sessionId) {
    throw new Error('Session expired!')
  }
}
function commitRotation(response: unknown, scope: AccountLinkScope) {
  assertAccountLinkScope(scope)
  const result = response as { success?: unknown; data?: unknown }
  if (result.success !== true) throw new Error('Account binding failed')
  if (
    !isAuthBundle(result.data) ||
    result.data.user.id !== scope.userId ||
    result.data.session.sid !== scope.sessionId
  ) {
    clearAuthentication(true)
    throw new Error('Invalid authentication rotation response')
  }
  applyAuthBundle(result.data)
}
export async function readAccountLinks(
  scope: AccountLinkScope
): Promise<AccountLinks> {
  assertAccountLinkScope(scope)
  const response = await api.get('/api/user/account-links', {
    authScope: scope,
    skipAuthRefresh: true,
    skipErrorHandler: true,
    timeout: 10000,
  })
  assertAccountLinkScope(scope)
  if (response.data?.success !== true) {
    throw new Error('Account bindings are unavailable')
  }
  return accountLinksSchema.parse(response.data.data)
}
export async function beginAccountBinding(
  scope: AccountLinkScope,
  selected: AccountProvider,
  expected: string,
  status: SystemStatus,
  origin: string
) {
  assertAccountLinkScope(scope)
  provider.parse(selected)
  version.parse(expected)
  return withAuthenticatedCookieWrite(async (config) => {
    assertAccountLinkScope(scope)
    const response = await api.post(
      `/api/user/account-links/${selected}/begin`,
      { expected_auth_version: expected },
      { ...config, skipErrorHandler: true }
    )
    assertAccountLinkScope(scope)
    if (response.data?.success !== true) {
      throw new Error('Account binding failed')
    }
    const result = bindStartSchema.parse(response.data.data)
    const authorizationUrl =
      selected === 'google'
        ? validateGoogleBrowserAuthorizationUrl(
            result.authorization_url,
            status.google_client_id,
            origin,
            false,
            status.google_browser_callback
          )
        : validateExternalBrowserAuthorizationUrl({
            provider: selected,
            value: result.authorization_url,
            clientId: status[`${selected}_client_id`],
            origin,
          })
    if (new URL(authorizationUrl).searchParams.get('state') !== result.state) {
      throw new Error('Account binding failed')
    }
    return { ...result, authorization_url: authorizationUrl }
  })
}
export async function finishAccountBinding(
  scope: AccountLinkScope,
  selected: AccountProvider,
  input: { state: string; code: string; iss?: string }
) {
  assertAccountLinkScope(scope)
  provider.parse(selected)
  if (
    !/^lmmo_[a-f0-9]{64}$/.test(input.state) ||
    !input.code ||
    input.code.length > 4096 ||
    (input.iss !== undefined &&
      input.iss !== ACCOUNT_PROVIDER_ISSUERS[selected]) ||
    (selected === 'discord' && input.iss !== ACCOUNT_PROVIDER_ISSUERS[selected])
  ) {
    throw new Error('Account binding failed')
  }
  return withAuthenticatedCookieWrite(async (config) => {
    assertAccountLinkScope(scope)
    const response = await api.post(
      `/api/user/account-links/${selected}/finish`,
      input,
      {
        ...config,
        skipErrorHandler: true,
      }
    )
    commitRotation(response.data, scope)
  })
}
export async function unlinkAccount(
  scope: AccountLinkScope,
  link: AccountLink,
  expected: string
) {
  assertAccountLinkScope(scope)
  linkSchema.parse(link)
  version.parse(expected)
  if (!link.active) throw new Error('Account binding failed')
  return withAuthenticatedCookieWrite(async (config) => {
    assertAccountLinkScope(scope)
    const response = await api.post(
      '/api/user/account-links/unlink',
      {
        issuer: link.issuer,
        subject: link.subject,
        expected_revision: link.revision,
        expected_auth_version: expected,
      },
      { ...config, skipErrorHandler: true }
    )
    commitRotation(response.data, scope)
  })
}
