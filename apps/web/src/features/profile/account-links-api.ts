/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

import { validateExternalBrowserAuthorizationUrl } from '@/features/auth/lib/external-oauth'
import {
  federatedIdSchema,
  federatedProviders,
  federatedStateSchema,
  consumeFederatedCeremonyForProvider,
  saveFederatedCeremony,
  validateFederatedStart,
} from '@/features/auth/lib/federated-oauth'
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
export type FixedAccountProvider = z.infer<typeof provider>
export type AccountProvider = FixedAccountProvider | `federated:${string}`
export const ACCOUNT_PROVIDER_ISSUERS: Record<FixedAccountProvider, string> = {
  google: 'https://accounts.google.com',
  github: 'https://github.com',
  linuxdo: 'https://connect.linux.do',
  discord: 'https://discord.com',
}
const linkFields = {
  issuer: z.string().min(1).max(512),
  subject: z.string().min(1).max(65536),
  revision,
  active: z.boolean(),
}
const linkSchema = z.union([
  z
    .object({ ...linkFields, subject: z.string().min(1).max(512) })
    .strict()
    .refine((row) => !row.issuer.startsWith('urn:lmm:federated:')),
  z
    .object({
      ...linkFields,
      provider_id: federatedIdSchema,
      slug: z.string().regex(/^[a-z0-9][a-z0-9-]{0,63}$/),
      name: z.string().min(1),
      kind: z.enum(['wechat', 'oidc', 'custom_oauth']),
      available: z.boolean(),
    })
    .strict()
    .refine((row) => row.issuer === `urn:lmm:federated:${row.provider_id}`),
])
const fixedDescriptor = z
  .object({ provider, issuer: z.string(), can_bind: z.boolean() })
  .strict()
  .refine((row) => row.issuer === ACCOUNT_PROVIDER_ISSUERS[row.provider])
const federatedDescriptor = z
  .object({
    provider: z.string().regex(/^federated:[1-9][0-9]{0,18}$/),
    provider_id: federatedIdSchema,
    issuer: z.string(),
    can_bind: z.boolean(),
    slug: z.string().regex(/^[a-z0-9][a-z0-9-]{0,63}$/),
    name: z.string().min(1),
    kind: z.enum(['wechat', 'oidc', 'custom_oauth']),
    icon: z.string(),
    qr_code_url: z.string(),
    native_federated: z.literal(true),
  })
  .strict()
  .refine(
    (row) =>
      row.provider === `federated:${row.provider_id}` &&
      row.issuer === `urn:lmm:federated:${row.provider_id}`
  )
export const accountLinksSchema = z
  .object({
    contract: z.literal('core.account-links.v1'),
    expected_auth_version: version,
    links: z.array(linkSchema).max(1101),
    providers: z
      .array(z.union([fixedDescriptor, federatedDescriptor]))
      .min(4)
      .max(1004)
      .refine(
        (rows) =>
          new Set(rows.map((row) => row.provider)).size === rows.length &&
          Object.keys(ACCOUNT_PROVIDER_ISSUERS).every((fixed) =>
            rows.some((row) => row.provider === fixed)
          ) &&
          new Set(
            rows
              .filter((row) => 'native_federated' in row)
              .map((row) => row.slug)
          ).size === rows.filter((row) => 'native_federated' in row).length
      ),
  })
  .strict()
export type AccountProviderDescriptor = z.infer<
  typeof accountLinksSchema
>['providers'][number]
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
function bindingFailure(value: unknown) {
  const envelope = z
    .object({
      code: z.string().optional(),
      error: z.object({ code: z.string().optional() }).optional(),
    })
    .safeParse(value)
  return Object.assign(new Error('Account binding failed'), {
    code: envelope.success
      ? (envelope.data.code ?? envelope.data.error?.code)
      : undefined,
  })
}
function commitRotation(response: unknown, scope: AccountLinkScope) {
  assertAccountLinkScope(scope)
  const result = response as { success?: unknown; data?: unknown }
  if (result.success !== true) throw bindingFailure(response)
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
    skipBusinessError: true,
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
  const native = selected.startsWith('federated:')
  if (!native) provider.parse(selected)
  version.parse(expected)
  const configured = native
    ? federatedProviders(status).find(
        (row) => `federated:${row.id}` === selected
      )
    : undefined
  if (native && !configured) throw new Error('Account binding failed')
  return withAuthenticatedCookieWrite(async (config) => {
    assertAccountLinkScope(scope)
    const response = await api.post(
      `/api/user/account-links/${selected}/begin`,
      { expected_auth_version: expected },
      { ...config, skipErrorHandler: true, skipBusinessError: true }
    )
    assertAccountLinkScope(scope)
    if (response.data?.success !== true) {
      throw bindingFailure(response.data)
    }
    if (configured) {
      const result = validateFederatedStart(
        configured,
        response.data.data,
        origin
      )
      saveFederatedCeremony(window.sessionStorage, configured, result, 'bind')
      return result
    }
    const fixed = provider.parse(selected)
    const result = bindStartSchema.parse(response.data.data)
    if (result.expires_at * 1000 <= Date.now()) {
      throw new Error('Account binding failed')
    }
    const authorizationUrl =
      fixed === 'google'
        ? validateGoogleBrowserAuthorizationUrl(
            result.authorization_url,
            status.google_client_id,
            origin,
            false,
            status.google_browser_callback
          )
        : validateExternalBrowserAuthorizationUrl({
            provider: fixed,
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
  const native = selected.startsWith('federated:')
  const fixed = native ? undefined : provider.parse(selected)
  if (
    !(native
      ? federatedStateSchema.safeParse(input.state).success
      : /^lmmo_[a-f0-9]{64}$/.test(input.state)) ||
    !input.code ||
    input.code.length > 4096 ||
    (fixed !== undefined &&
      input.iss !== undefined &&
      input.iss !== ACCOUNT_PROVIDER_ISSUERS[fixed]) ||
    (selected === 'discord' && input.iss !== ACCOUNT_PROVIDER_ISSUERS[selected])
  ) {
    throw new Error('Account binding failed')
  }
  if (native) {
    consumeFederatedCeremonyForProvider(
      window.sessionStorage,
      input.state,
      selected.slice('federated:'.length),
      'bind'
    )
  }
  return withAuthenticatedCookieWrite(async (config) => {
    assertAccountLinkScope(scope)
    const response = await api.post(
      `/api/user/account-links/${selected}/finish`,
      input,
      {
        ...config,
        skipErrorHandler: true,
        skipBusinessError: true,
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
      { ...config, skipErrorHandler: true, skipBusinessError: true }
    )
    commitRotation(response.data, scope)
  })
}
