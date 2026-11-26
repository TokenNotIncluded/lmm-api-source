/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

import {
  isCurrentNativeUsersScope,
  nativeUsersOptions,
  type NativeUsersScope,
} from '@/features/native-users/access'
import { api } from '@/lib/api'

const optionKeys = [
  'ModelRatio',
  'CompletionRatio',
  'CacheRatio',
  'CreateCacheRatio',
  'AudioRatio',
  'AudioCompletionRatio',
  'ImageRatio',
  'ModelPrice',
  'ModelPriceLock',
  'GroupRatio',
  'GroupGroupRatio',
  'group_ratio_setting.group_ratio',
  'group_ratio_setting.group_special_usable_group',
  'group_ratio_setting.group_warnings',
  'UserUsableGroups',
  'DefaultUseAutoGroup',
  'billing_setting.billing_mode',
  'billing_setting.billing_expr',
  'ServiceTierPricingCatalog',
  'ServiceTierPricingPolicy',
  'dynamic_pricing_setting.base_price_usd_per_million',
  'USDExchangeRate',
] as const

const rawText = (max: number) =>
  z
    .string()
    .max(max)
    .refine((value) => !value.includes('\0'))
const snapshotSchema = z
  .object({
    contract: z.literal('core.legacy-pricing-snapshot.v1'),
    purpose: z.literal('reference_only'),
    source_sha256: z.string().regex(/^[a-f0-9]{64}$/),
    observed_at: z.number().int().min(1).max(253_402_300_799),
    options: z
      .array(
        z
          .object({
            key: z.enum(optionKeys),
            value: rawText(512 * 1024),
          })
          .strict()
      )
      .max(optionKeys.length),
    enabled_channels: z
      .array(
        z
          .object({
            id: z.number().int().positive().safe(),
            name: rawText(256),
            models: rawText(128 * 1024),
            groups: rawText(8192),
            model_mapping: rawText(128 * 1024),
          })
          .strict()
      )
      .max(4096),
    unsupported: z
      .array(
        z
          .object({
            model: rawText(1024).refine(Boolean),
            group: rawText(1024).refine(Boolean),
            reason: z
              .string()
              .min(1)
              .max(160)
              .regex(/^[a-z0-9_]+$/),
            source_channel_ids: z
              .array(z.number().int().positive().safe())
              .max(4096),
          })
          .strict()
      )
      .max(10_000),
  })
  .strict()
  .refine((snapshot) => {
    const channels = new Set(snapshot.enabled_channels.map((item) => item.id))
    return (
      channels.size === snapshot.enabled_channels.length &&
      new Set(snapshot.options.map((item) => item.key)).size ===
        snapshot.options.length &&
      new Set(
        snapshot.unsupported.map((item) =>
          JSON.stringify([item.model, item.group])
        )
      ).size === snapshot.unsupported.length &&
      snapshot.unsupported.every((item) =>
        item.source_channel_ids.every((id) => channels.has(id))
      )
    )
  })

export type PricingSourceSnapshot = z.infer<typeof snapshotSchema>

export function parsePricingSourceSnapshot(
  value: unknown
): PricingSourceSnapshot | null {
  if (
    new TextEncoder().encode(JSON.stringify(value)).length >
    2 * 1024 * 1024 + 1024
  ) {
    throw new Error('Invalid original pricing snapshot')
  }
  return z
    .object({
      success: z.literal(true),
      data: snapshotSchema.nullable(),
    })
    .strict()
    .parse(value).data
}

export async function readPricingSourceSnapshot(
  scope: NativeUsersScope,
  signal?: AbortSignal
): Promise<PricingSourceSnapshot | null> {
  if (!isCurrentNativeUsersScope(scope)) throw new Error('Session changed')
  const response = await api.get<unknown>(
    '/core/v1/admin/routing/pricing-source-snapshot',
    nativeUsersOptions(scope, signal)
  )
  if (!isCurrentNativeUsersScope(scope) || signal?.aborted) {
    throw new Error('Session changed')
  }
  return parsePricingSourceSnapshot(response.data)
}

/** The raw option values remain strings; never parse/round their numeric lexemes. */
export function serializePricingSourceSnapshot(
  snapshot: PricingSourceSnapshot
): string {
  return `${JSON.stringify(snapshot, null, 2)}\n`
}

/** Old deployment's original group names and descriptions, for reference only. */
export function originalGroupDescriptions(
  snapshot: PricingSourceSnapshot | null | undefined
) {
  const value = snapshot?.options.find(
    (option) => option.key === 'UserUsableGroups'
  )?.value
  if (!value) return new Map<string, string>()
  try {
    const parsed: unknown = JSON.parse(value)
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed))
      return new Map<string, string>()
    return new Map(
      Object.entries(parsed).filter(
        (entry): entry is [string, string] =>
          typeof entry[1] === 'string' && entry[1].trim() !== ''
      )
    )
  } catch {
    return new Map<string, string>()
  }
}
