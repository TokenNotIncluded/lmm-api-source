/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { isAxiosError } from 'axios'
import { z } from 'zod'

import { api, type ApiRequestConfig } from '@/lib/api'

export type SettingsScope = NonNullable<ApiRequestConfig['authScope']>
const groupName = z
  .string()
  .refine(
    (value) =>
      Array.from(value).length <= 64 &&
      /^[\p{L}\p{M}\p{N}\p{S}._/()[\]-]+$/u.test(value) &&
      Array.from(value).every(
        (char) =>
          char.charCodeAt(0) > 127 || /^[A-Za-z0-9._/()[\]-]$/.test(char)
      )
  )
const limit = z.number().int().min(1).max(100_000_000)
export const requestLimitPolicySchema = z
  .object({
    enabled: z.boolean(),
    window_seconds: z.number().int().min(1).max(86400),
    user_limit: limit.nullable(),
    group_limits: z
      .record(groupName, limit)
      .refine((groups) => Object.keys(groups).length <= 1024),
  })
  .strict()
  .refine(
    (policy) =>
      !policy.enabled ||
      policy.user_limit !== null ||
      Object.keys(policy.group_limits).length > 0
  )
export const requestLimitSnapshotSchema = z.object({
  version: z.literal(1),
  revision: z.number().int().positive(),
  policy: requestLimitPolicySchema,
})
export const autoGroupOrderSchema = z
  .array(groupName)
  .max(1024)
  .refine((groups) => new Set(groups).size === groups.length)
const autoGroupSnapshotSchema = z.object({
  version: z.literal(1),
  revision: z.number().int().positive(),
  groups: autoGroupOrderSchema,
})
export type RequestLimitPolicy = z.infer<typeof requestLimitPolicySchema>
export type RequestLimitSnapshot = z.infer<typeof requestLimitSnapshotSchema>
export type AutoGroupSnapshot = z.infer<typeof autoGroupSnapshotSchema>

const options = (authScope: SettingsScope): ApiRequestConfig => ({
  authScope,
  skipErrorHandler: true,
  skipBusinessError: true,
  disableDuplicate: true,
  timeout: 10_000,
})
export function configurationErrorKey(error: unknown) {
  return isAxiosError(error) && error.response?.status === 409
    ? 'Settings changed. Reload the current configuration before saving again.'
    : 'Failed to update setting'
}
export async function getRequestLimits(scope: SettingsScope) {
  const response = await api.get(
    '/core/v1/admin/request-limits',
    options(scope)
  )
  return requestLimitSnapshotSchema.parse(response.data)
}
export async function saveRequestLimits(
  scope: SettingsScope,
  expectedRevision: number,
  policy: RequestLimitPolicy
) {
  const response = await api.put(
    '/core/v1/admin/request-limits',
    {
      expected_revision: expectedRevision,
      policy: requestLimitPolicySchema.parse(policy),
    },
    options(scope)
  )
  return requestLimitSnapshotSchema.parse(response.data)
}
export async function getAutoGroupOrder(scope: SettingsScope) {
  const response = await api.get(
    '/core/v1/admin/routing/auto-groups',
    options(scope)
  )
  return autoGroupSnapshotSchema.parse(response.data)
}
export async function saveAutoGroupOrder(
  scope: SettingsScope,
  expectedRevision: number,
  groups: string[]
) {
  const response = await api.put(
    '/core/v1/admin/routing/auto-groups',
    {
      expected_revision: expectedRevision,
      groups: autoGroupOrderSchema.parse(groups),
    },
    options(scope)
  )
  return autoGroupSnapshotSchema.parse(response.data)
}
