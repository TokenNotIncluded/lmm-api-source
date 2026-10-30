/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

import { api } from '@/lib/api'
import { ROLE } from '@/lib/roles'
import { useAuthStore } from '@/stores/auth-store'

export type NativeVersion = { routing: number; pricing: number }
export type NativeCredentialRef = { id: number; revision: number }
export type NativeContext = {
  upstream_id: number
  upstream_model: string
  protocol: string
  endpoint: string
  max_input_tokens: number
  max_output_tokens: number
  contract_revision: string
  [key: string]: unknown
}
export type NativeTarget = {
  id: number
  upstream_id: number
  upstream_model: string
  enabled: boolean
  priority: number
  weight: number
  speeds: string[]
  payload: string | Record<string, unknown>
  timeout_ms: number
}
export type NativeModelRoute = {
  id: number
  model: string
  group: string
  enabled: boolean
  speeds: string[]
  targets: NativeTarget[]
  failover: {
    max_attempts: number
    retry_on: string[]
    allow_replay_after_send: boolean
  }
}
export type NativeUpstream = {
  id: number
  endpoint: string
  protocol: string
  credential: NativeCredentialRef
  enabled: boolean
}
export type NativeVendor = { id: number; name: string; description: string }
export type NativeModelMetadata = {
  display_name: string
  description: string
  vendor_id: number | null
}
export type NativePrices = {
  version: number
  rules: Array<Record<string, unknown>>
}
export type NativeConfiguration = {
  version: NativeVersion
  routing: {
    version: number
    upstreams: NativeUpstream[]
    routes: NativeModelRoute[]
    groups: Array<{ name: string; enabled: boolean }>
    aliases: unknown[]
  }
  prices: NativePrices
  bindings: {
    names: Record<string, string>
    contexts: NativeContext[]
    vendors: Record<string, NativeVendor>
    models: Record<string, NativeModelMetadata>
    discoveries: Record<string, { models: string[]; observed_at: number }>
    price_drafts: Record<string, NativePriceDraft>
  }
  secret_storage_enabled: boolean
}
export type NativePriceDraft = {
  id: number
  name: string
  base_pricing_version: number
  prices: NativePrices
  updated_by: number
  updated_at: number
}
export type NativeKeyList = {
  version: NativeVersion
  active_reference: NativeCredentialRef
  keys: Array<{
    reference: NativeCredentialRef
    label: string
    created_at: string
  }>
  plaintext_available: false
}

/** Reject rounded identifiers and monetary values before any mutation. */
export function assertNativeIntegers(value: unknown): void {
  if (typeof value === 'number' && !Number.isSafeInteger(value)) {
    throw new Error(
      'This configuration contains a number that cannot be edited safely in this browser.'
    )
  }
  if (Array.isArray(value)) value.forEach(assertNativeIntegers)
  else if (value !== null && typeof value === 'object') {
    Object.values(value).forEach(assertNativeIntegers)
  }
}
export function parseNativeDocument(text: string): Record<string, unknown> {
  const value: unknown = JSON.parse(text)
  if (!value || Array.isArray(value) || typeof value !== 'object') {
    throw new Error('Configuration must be a JSON object.')
  }
  assertNativeIntegers(value)
  return value as Record<string, unknown>
}
export type NativeRoutingScope = {
  userId: number
  sessionId: string
  role: number
}
export function readNativeRoutingScope(
  auth = useAuthStore.getState().auth
): NativeRoutingScope | undefined {
  const user = auth.user
  const session = auth.session
  if (
    !user ||
    !Number.isSafeInteger(user.id) ||
    user.id <= 0 ||
    !session?.sid ||
    (user.role !== ROLE.ADMIN && user.role !== ROLE.SUPER_ADMIN)
  ) {
    return undefined
  }
  return { userId: user.id, sessionId: session.sid, role: user.role }
}
function currentScope(scope: NativeRoutingScope): void {
  const current = readNativeRoutingScope()
  if (
    !current ||
    current.userId !== scope.userId ||
    current.sessionId !== scope.sessionId ||
    current.role !== scope.role
  ) {
    throw new Error(
      'The administrative session changed. Reload before continuing.'
    )
  }
}
const versionSchema = z.object({
  routing: z.number().int().positive().max(Number.MAX_SAFE_INTEGER),
  pricing: z.number().int().positive().max(Number.MAX_SAFE_INTEGER),
})
export async function nativeRequest<T>(
  scope: NativeRoutingScope,
  method: 'GET' | 'POST' | 'PUT' | 'DELETE',
  path: string,
  data?: unknown
): Promise<T> {
  currentScope(scope)
  if (data !== undefined) assertNativeIntegers(data)
  const response = await api.request<{
    success: boolean
    message?: string
    data: T
  }>({
    method,
    url: path,
    data,
    authScope: { userId: scope.userId, sessionId: scope.sessionId },
    skipAuthRefresh: true,
    skipBusinessError: true,
    skipErrorHandler: true,
    disableDuplicate: true,
    timeout: 15_000,
  })
  currentScope(scope)
  if (
    response.data?.success !== true ||
    !Object.hasOwn(response.data, 'data')
  ) {
    throw new Error(
      response.data?.message || 'The operation did not return a valid result.'
    )
  }
  assertNativeIntegers(response.data.data)
  if (method !== 'GET' && !path.endsWith('/preview')) {
    const input = data as { expected_version?: unknown }
    const result = response.data.data as { version?: unknown }
    const expected = versionSchema.parse(input?.expected_version)
    const version = versionSchema.safeParse(result?.version)
    if (
      !version.success ||
      version.data.routing !== expected.routing + 1 ||
      version.data.pricing < expected.pricing
    ) {
      throw new Error('The save result is unknown. Reload before continuing.')
    }
  }
  return response.data.data
}
export const NATIVE_ROOT = '/core/v1/admin/routing'
const configurationSchema = z.object({
  version: versionSchema,
  routing: z.object({
    version: z.number(),
    upstreams: z.array(
      z.object({
        id: z.number(),
        endpoint: z.string(),
        protocol: z.string(),
        credential: z.object({ id: z.number(), revision: z.number() }),
        enabled: z.boolean(),
      })
    ),
    routes: z.array(
      z.object({
        id: z.number(),
        model: z.string(),
        group: z.string(),
        enabled: z.boolean(),
        speeds: z.array(z.string()),
        targets: z.array(
          z.object({
            id: z.number(),
            upstream_id: z.number(),
            upstream_model: z.string(),
            enabled: z.boolean(),
            priority: z.number(),
            weight: z.number(),
            speeds: z.array(z.string()),
            payload: z.union([z.string(), z.record(z.string(), z.unknown())]),
            timeout_ms: z.number(),
          })
        ),
        failover: z.object({
          max_attempts: z.number(),
          retry_on: z.array(z.string()),
          allow_replay_after_send: z.boolean(),
        }),
      })
    ),
    groups: z.array(z.object({ name: z.string(), enabled: z.boolean() })),
    aliases: z.array(z.unknown()),
  }),
  prices: z.object({
    version: z.number(),
    rules: z.array(z.record(z.string(), z.unknown())),
  }),
  bindings: z.object({
    names: z.record(z.string(), z.string()),
    contexts: z.array(
      z
        .object({
          upstream_id: z.number(),
          upstream_model: z.string(),
          protocol: z.string(),
          endpoint: z.string(),
          max_input_tokens: z.number(),
          max_output_tokens: z.number(),
          contract_revision: z.string(),
        })
        .passthrough()
    ),
    vendors: z.record(
      z.string(),
      z.object({ id: z.number(), name: z.string(), description: z.string() })
    ),
    models: z.record(
      z.string(),
      z.object({
        display_name: z.string(),
        description: z.string(),
        vendor_id: z.number().nullable(),
      })
    ),
    discoveries: z.record(
      z.string(),
      z.object({
        models: z.array(z.string()),
        observed_at: z.number().int().min(1).max(253_402_300_799),
      })
    ),
    price_drafts: z.record(
      z.string(),
      z.object({
        id: z.number(),
        name: z.string(),
        base_pricing_version: z.number(),
        prices: z.object({
          version: z.number(),
          rules: z.array(z.record(z.string(), z.unknown())),
        }),
        updated_by: z.number(),
        updated_at: z.number().int().min(1).max(253_402_300_799),
      })
    ),
  }),
  secret_storage_enabled: z.boolean(),
})
export async function getNativeConfiguration(
  scope: NativeRoutingScope
): Promise<NativeConfiguration> {
  const data = await nativeRequest<unknown>(
    scope,
    'GET',
    `${NATIVE_ROOT}/configuration`
  )
  const parsed = configurationSchema.parse(data)
  if (
    parsed.version.routing !== parsed.routing.version ||
    parsed.version.pricing !== parsed.prices.version
  ) {
    throw new Error('The routing snapshot changed. Reload before continuing.')
  }
  return parsed as NativeConfiguration
}

const keyListSchema = z.object({
  version: versionSchema,
  active_reference: z.object({
    id: z.number().int().positive(),
    revision: z.number().int().positive(),
  }),
  keys: z.array(
    z.object({
      reference: z.object({
        id: z.number().int().positive(),
        revision: z.number().int().positive(),
      }),
      label: z.string(),
      created_at: z.string(),
    })
  ),
  plaintext_available: z.literal(false),
})
export async function getNativeKeys(
  scope: NativeRoutingScope,
  id: number
): Promise<NativeKeyList> {
  assertNativeIntegers(id)
  if (id <= 0) throw new Error('Invalid channel ID')
  return keyListSchema.parse(
    await nativeRequest<unknown>(
      scope,
      'GET',
      `${NATIVE_ROOT}/channels/${id}/keys`
    )
  )
}
