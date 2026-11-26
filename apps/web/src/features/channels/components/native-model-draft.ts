/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

import { newNativeTargetTimeoutMs } from '../lib/native-target-timeout'
import { parseNativeDocument, type NativeConfiguration } from '../native-api'

const numberDraft = z.union([z.number().int().safe(), z.string()])
const targetSchema = z
  .object({
    id: z.number().int().positive().safe(),
    upstream_id: z.number().int().positive().safe(),
    upstream_model: z.string(),
    enabled: z.boolean(),
    priority: numberDraft,
    weight: numberDraft,
    timeout_ms: numberDraft,
    speeds: z.array(z.string()),
  })
  .passthrough()
const modelDraftSchema = z
  .object({
    route: z
      .object({
        id: z.number().int().positive().safe(),
        model: z.string(),
        group: z.string(),
        enabled: z.boolean(),
        targets: z.array(targetSchema),
        speeds: z.array(z.string()),
        failover: z.object({
          max_attempts: numberDraft,
          retry_on: z.array(z.string()),
          allow_replay_after_send: z.boolean(),
        }),
      })
      .passthrough(),
    metadata: z
      .object({
        display_name: z.string(),
        tags: z.array(z.string()).optional(),
        description: z.string(),
        vendor_id: z.number().nullable(),
      })
      .passthrough(),
    prices: z.array(
      z
        .object({
          id: z.number().int().positive().safe(),
          model: z.string(),
          group: z.string(),
          speed: z.string(),
          rates: z
            .object({
              input: numberDraft,
              output: numberDraft,
              cache_read: numberDraft,
              cache_write: numberDraft,
              request: numberDraft,
            })
            .passthrough(),
        })
        .passthrough()
    ),
    contexts: z.array(z.record(z.string(), z.unknown())),
  })
  .passthrough()

export type NativeModelDraft = z.infer<typeof modelDraftSchema>
export type NativeTargetDraft = z.infer<typeof targetSchema>

export function readNativeModelDraft(
  document: string
): NativeModelDraft | null {
  try {
    const result = modelDraftSchema.safeParse(parseNativeDocument(document))
    return result.success ? result.data : null
  } catch {
    return null
  }
}

export function modelIdentity(
  draft: NativeModelDraft,
  key: 'model' | 'group',
  value: string
): NativeModelDraft {
  return {
    ...draft,
    route: { ...draft.route, [key]: value },
    prices: draft.prices.map((price) => ({ ...price, [key]: value })),
  }
}

function matchesContext(
  context: Record<string, unknown>,
  target: NativeTargetDraft
) {
  return (
    context.upstream_id === target.upstream_id &&
    context.upstream_model === target.upstream_model
  )
}

/** Reuse only an exact supplier/model contract; never transfer another model's limits. */
export function replaceModelTargets(
  draft: NativeModelDraft,
  targets: NativeTargetDraft[],
  configuration: NativeConfiguration
): NativeModelDraft {
  const contexts: NativeModelDraft['contexts'] = []
  for (const target of targets) {
    if (contexts.some((context) => matchesContext(context, target))) continue
    const current =
      draft.contexts.find((context) => matchesContext(context, target)) ??
      configuration.bindings.contexts.find((context) =>
        matchesContext(context, target)
      )
    if (current) {
      contexts.push({ ...current })
      continue
    }
    const upstream = configuration.routing.upstreams.find(
      (item) => item.id === target.upstream_id
    )
    if (upstream) {
      contexts.push({
        upstream_id: upstream.id,
        upstream_model: target.upstream_model,
        endpoint: upstream.endpoint,
        protocol: upstream.protocol,
        max_input_tokens: '',
        max_output_tokens: '',
        contract_revision: '',
      })
    }
  }
  return { ...draft, route: { ...draft.route, targets }, contexts }
}

export function newModelTarget(
  draft: NativeModelDraft,
  configuration: NativeConfiguration,
  upstreamId: number
): NativeTargetDraft {
  return {
    id:
      Math.max(
        0,
        ...configuration.routing.routes.flatMap((route) =>
          route.targets.map((target) => target.id)
        ),
        ...draft.route.targets.map((target) => target.id)
      ) + 1,
    upstream_id: upstreamId,
    upstream_model: draft.route.model,
    enabled: true,
    priority: 0,
    weight: 1,
    timeout_ms: newNativeTargetTimeoutMs(configuration),
    speeds: [...draft.route.speeds],
    payload: 'adapt',
  }
}

/** Convert ordinary dollar input to exact micro-USD without floating-point multiplication. */
export function usdToMicros(value: string): number | null {
  if (!/^\d+(?:\.\d{0,6})?$/.test(value)) return null
  const [whole, fraction = ''] = value.split('.')
  const micro = BigInt(whole) * 1_000_000n + BigInt(fraction.padEnd(6, '0'))
  return micro <= BigInt(Number.MAX_SAFE_INTEGER) ? Number(micro) : null
}
export function microsToUsd(value: number | string): string {
  if (typeof value === 'string') return value
  const micro = BigInt(value)
  const fraction = (micro % 1_000_000n)
    .toString()
    .padStart(6, '0')
    .replace(/0+$/, '')
  return (micro / 1_000_000n).toString() + (fraction ? `.${fraction}` : '')
}
