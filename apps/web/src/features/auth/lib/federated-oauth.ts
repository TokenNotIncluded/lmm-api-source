/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

import { INITIAL_LOGIN_INTENT } from '@/lib/auth-login-intent'

import type { SystemStatus } from '../types'

export const federatedIdSchema = z
  .string()
  .regex(/^[1-9][0-9]{0,18}$/)
  .refine((value) => BigInt(value) <= 9223372036854775807n)
export const federatedStateSchema = z.string().regex(/^lmmfe_[a-f0-9]{64}$/)
const slugSchema = z.string().regex(/^[a-z0-9][a-z0-9-]{0,63}$/)
const catalogFields = {
  id: federatedIdSchema,
  slug: slugSchema,
  name: z.string().min(1),
  icon: z.string(),
  revision: federatedIdSchema,
  client_id: z.string(),
  authorization_endpoint: z.string(),
  scopes: z.string(),
  qr_code_url: z.string(),
  native_federated: z.literal(true),
  registration_enabled: z.boolean().optional(),
}
const federatedProviderSchema = z.discriminatedUnion('kind', [
  z
    .object({
      ...catalogFields,
      kind: z.literal('oidc'),
      client_id: z.string().min(1),
      authorization_endpoint: z.string().min(1),
    })
    .strict(),
  z
    .object({
      ...catalogFields,
      kind: z.literal('custom_oauth'),
      client_id: z.string().min(1),
      authorization_endpoint: z.string().min(1),
    })
    .strict(),
  z.object({ ...catalogFields, kind: z.literal('wechat') }).strict(),
])
export type FederatedProvider = z.infer<typeof federatedProviderSchema>
const federatedStartSchema = z
  .object({
    authorization_url: z.string(),
    state: federatedStateSchema,
    expires_at: z.number().int().positive(),
  })
  .strict()
export type FederatedStart = z.infer<typeof federatedStartSchema>
const ceremonySchema = z
  .object({
    state: federatedStateSchema,
    providerId: federatedIdSchema,
    slug: slugSchema,
    intent: z.enum(['login', 'bind']),
    expiresAt: z.number().int().positive(),
    loginIntentVersion: z.string().min(1).max(128).optional(),
  })
  .strict()
export type FederatedCeremony = z.infer<typeof ceremonySchema>
export type FederatedStorage = Pick<
  Storage,
  'getItem' | 'setItem' | 'removeItem'
>
const prefix = 'lmm_federated:'
export function federatedProviders(
  status: SystemStatus | null | undefined
): FederatedProvider[] {
  if (
    (status?.native_federated_auth ?? status?.data?.native_federated_auth) !==
    true
  ) {
    return []
  }
  const providers = z
    .array(federatedProviderSchema)
    .max(1000)
    .parse(
      status?.federated_oauth_providers ??
        status?.data?.federated_oauth_providers ??
        []
    )
  if (
    new Set(providers.map((p) => p.id)).size !== providers.length ||
    new Set(providers.map((p) => p.slug)).size !== providers.length
  ) {
    throw new Error('Invalid federated catalog')
  }
  return providers
}
export function validateFederatedStart(
  provider: FederatedProvider,
  value: unknown,
  origin: string
): FederatedStart {
  const flow = federatedStartSchema.parse(value)
  if (flow.expires_at * 1000 <= Date.now()) {
    throw new Error('Login flow expired. Please sign in again.')
  }
  if (provider.kind === 'wechat') {
    if (flow.authorization_url !== '') {
      throw new Error('Invalid federated authorization URL')
    }
    return flow
  }
  const endpoint = new URL(provider.authorization_endpoint)
  const url = new URL(flow.authorization_url)
  if (
    endpoint.protocol !== 'https:' ||
    url.protocol !== 'https:' ||
    endpoint.username ||
    endpoint.password ||
    endpoint.hash ||
    url.username ||
    url.password ||
    url.hash ||
    endpoint.origin !== url.origin ||
    endpoint.pathname !== url.pathname
  ) {
    throw new Error('Untrusted OAuth authorization endpoint')
  }
  for (const key of [
    'client_id',
    'redirect_uri',
    'response_type',
    'state',
    'scope',
  ]) {
    if (url.searchParams.getAll(key).length !== 1) {
      throw new Error('Invalid federated authorization URL')
    }
  }
  if (
    url.searchParams.get('client_id') !== provider.client_id ||
    url.searchParams.get('redirect_uri') !==
      `${origin}/oauth/${provider.slug}` ||
    url.searchParams.get('state') !== flow.state ||
    url.searchParams.get('response_type') !== 'code'
  ) {
    throw new Error('Invalid federated authorization URL')
  }
  if (
    provider.kind === 'oidc' &&
    (url.searchParams.getAll('nonce').length !== 1 ||
      !url.searchParams.get('nonce'))
  ) {
    throw new Error('Invalid federated authorization URL')
  }
  for (const key of ['nonce', 'code_challenge', 'code_challenge_method']) {
    if (url.searchParams.getAll(key).length > 1) {
      throw new Error('Invalid federated authorization URL')
    }
  }
  if (
    (url.searchParams.has('code_challenge') ||
      url.searchParams.has('code_challenge_method')) &&
    (url.searchParams.get('code_challenge_method') !== 'S256' ||
      !/^[A-Za-z0-9_-]{43}$/.test(url.searchParams.get('code_challenge') ?? ''))
  ) {
    throw new Error('Invalid federated authorization URL')
  }
  // Preserve the exact Core URL, including provider-specific query parameters.
  return flow
}
export function saveFederatedCeremony(
  storage: FederatedStorage,
  provider: FederatedProvider,
  flow: FederatedStart,
  intent: FederatedCeremony['intent'],
  loginIntentVersion = INITIAL_LOGIN_INTENT
) {
  const ceremony = ceremonySchema.parse({
    state: flow.state,
    providerId: provider.id,
    slug: provider.slug,
    intent,
    expiresAt: flow.expires_at,
    ...(intent === 'login' ? { loginIntentVersion } : {}),
  })
  const encoded = JSON.stringify(ceremony)
  storage.setItem(prefix + flow.state, encoded)
  if (storage.getItem(prefix + flow.state) !== encoded) {
    throw new Error('OAuth storage unavailable')
  }
  return ceremony
}
function loadFederatedCeremony(
  storage: FederatedStorage,
  state: string,
  intent: FederatedCeremony['intent']
): FederatedCeremony {
  federatedStateSchema.parse(state)
  const flow = ceremonySchema.parse(
    JSON.parse(storage.getItem(prefix + state) ?? 'null')
  )
  if (
    flow.state !== state ||
    flow.intent !== intent ||
    (intent === 'login' && !flow.loginIntentVersion) ||
    flow.expiresAt * 1000 <= Date.now()
  ) {
    throw new Error('Login flow expired. Please sign in again.')
  }
  return flow
}
export function readFederatedCeremony(
  storage: FederatedStorage,
  state: string,
  slug: string,
  intent: FederatedCeremony['intent']
) {
  const flow = loadFederatedCeremony(storage, state, intent)
  if (flow.slug !== slug) {
    throw new Error('Login flow expired. Please sign in again.')
  }
  return flow
}
export function consumeFederatedCeremony(
  storage: FederatedStorage,
  state: string,
  slug: string,
  intent: FederatedCeremony['intent']
) {
  const flow = readFederatedCeremony(storage, state, slug, intent)
  storage.removeItem(prefix + state)
  return flow
}
export function consumeFederatedCeremonyForProvider(
  storage: FederatedStorage,
  state: string,
  expectedProviderId: string,
  intent: FederatedCeremony['intent']
) {
  federatedIdSchema.parse(expectedProviderId)
  const flow = loadFederatedCeremony(storage, state, intent)
  if (flow.providerId !== expectedProviderId) {
    throw new Error('Account binding failed')
  }
  storage.removeItem(prefix + state)
  return flow
}
export function forgetFederatedCeremony(
  storage: FederatedStorage,
  state: string
) {
  storage.removeItem(prefix + state)
}
