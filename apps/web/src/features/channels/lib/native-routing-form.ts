/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

import { parseNativeDocument, type NativeContext } from '../native-api'

export const nativeProtocols = [
  'openai_chat',
  'openai_responses',
  'anthropic_messages',
  'gemini',
  'system_one',
  'openai_embeddings',
  'jina_rerank',
  'deepseek_search',
  'deepseek_fetch',
] as const

const numberDraft = z.union([z.number().int().safe(), z.string()])

const contextSchema = z
  .object({
    upstream_id: z.number(),
    upstream_model: z.string(),
    protocol: z.string(),
    endpoint: z.string(),
    max_input_tokens: numberDraft,
    max_output_tokens: numberDraft,
    contract_revision: z.string(),
  })
  .passthrough()

export const targetFieldsSchema = z
  .object({
    upstream_model: z.string(),
    priority: numberDraft,
    weight: numberDraft,
    speeds: z.array(z.string()),
    timeout_ms: numberDraft,
    payload: z.union([z.string(), z.record(z.string(), z.unknown())]),
    enabled: z.boolean().optional(),
    client_protocols: z.array(z.string()).nullish(),
  })
  .passthrough()

export type NativeTargetFieldsValue = z.infer<typeof targetFieldsSchema>
export const channelDraftSchema = z
  .object({
    id: z.number(),
    name: z.string(),
    tag: z.string().optional(),
    endpoint: z.string(),
    protocol: z.string(),
    enabled: z.boolean(),
    contexts: z.array(contextSchema),
    attachments: z.array(targetFieldsSchema.extend({ route_id: z.number() })),
  })
  .passthrough()
export type NativeChannelDraft = z.infer<typeof channelDraftSchema>

export const modelDraftSchema = z
  .object({
    route: z
      .object({
        id: z.number(),
        model: z.string(),
        group: z.string(),
        enabled: z.boolean(),
        speeds: z.array(z.string()),
        targets: z.array(
          targetFieldsSchema.extend({
            id: z.number(),
            upstream_id: z.number(),
            enabled: z.boolean(),
          })
        ),
        failover: z.object({
          max_attempts: z.number(),
          retry_on: z.array(z.string()),
          allow_replay_after_send: z.boolean(),
        }),
      })
      .passthrough(),
    metadata: z
      .object({
        display_name: z.string(),
        description: z.string(),
        vendor_id: z.number().nullable(),
      })
      .passthrough(),
    prices: z.array(
      z
        .object({
          id: z.number(),
          model: z.string(),
          group: z.string(),
          speed: z.string(),
          rates: z.object({
            input: z.number(),
            output: z.number(),
            cache_read: z.number(),
            cache_write: z.number(),
            request: z.number(),
          }),
          group_multiplier_ppm: z.number(),
          speed_multiplier_ppm: z.number(),
          billing_mode: z.string().optional(),
        })
        .passthrough()
    ),
    contexts: z.array(contextSchema),
  })
  .passthrough()
export type NativeModelDraft = z.infer<typeof modelDraftSchema>

export function readModelDraft(document: string): NativeModelDraft | null {
  try {
    return modelDraftSchema.parse(parseNativeDocument(document))
  } catch {
    return null
  }
}

export function updateModelIdentity(
  draft: NativeModelDraft,
  field: 'model' | 'group',
  value: string
): NativeModelDraft {
  return {
    ...draft,
    route: { ...draft.route, [field]: value },
    prices: draft.prices.map((price) => ({ ...price, [field]: value })),
    metadata:
      field === 'model' &&
      (!draft.metadata.display_name ||
        draft.metadata.display_name === draft.route.model)
        ? { ...draft.metadata, display_name: value }
        : draft.metadata,
  }
}

export function readChannelDraft(document: string): NativeChannelDraft | null {
  try {
    return channelDraftSchema.parse(parseNativeDocument(document))
  } catch {
    return null
  }
}

export function newNativeContext(
  upstream: { id: number; endpoint: string; protocol: string },
  model: string
): NativeContext {
  return {
    upstream_id: upstream.id,
    upstream_model: model,
    endpoint: upstream.endpoint,
    protocol: upstream.protocol,
    max_input_tokens: 0,
    max_output_tokens: 0,
    contract_revision: '',
  }
}

/** A new supplier model needs its own contract; an exact existing model can share one. */
export function updateChannelAttachment(
  draft: NativeChannelDraft,
  routeId: number,
  patch: Partial<NativeTargetFieldsValue>,
  publishedContexts: readonly NativeContext[] = []
): NativeChannelDraft {
  const previous = draft.attachments.find((item) => item.route_id === routeId)
  if (!previous) return draft
  const attachments = draft.attachments.map((item) =>
    item.route_id === routeId ? { ...item, ...patch } : item
  )
  let contexts = draft.contexts
  if (
    patch.upstream_model !== undefined &&
    patch.upstream_model !== previous.upstream_model
  ) {
    const destination = contexts.find(
      (context) => context.upstream_model === patch.upstream_model
    )
    if (!destination) {
      const published = publishedContexts.find(
        (context) =>
          context.upstream_id === draft.id &&
          context.upstream_model === patch.upstream_model
      )
      contexts = [
        ...contexts,
        published
          ? { ...published, endpoint: draft.endpoint, protocol: draft.protocol }
          : newNativeContext(draft, patch.upstream_model),
      ]
    }
    if (
      !attachments.some(
        (item) => item.upstream_model === previous.upstream_model
      )
    ) {
      contexts = contexts.filter(
        (context) => context.upstream_model !== previous.upstream_model
      )
    }
  }
  return { ...draft, attachments, contexts }
}

export function toggleChannelAttachment(
  draft: NativeChannelDraft,
  route: { id: number; model: string; speeds: string[] },
  enabled: boolean
): NativeChannelDraft {
  if (!enabled) {
    const attachments = draft.attachments.filter(
      (item) => item.route_id !== route.id
    )
    const removed = draft.attachments.find((item) => item.route_id === route.id)
    return {
      ...draft,
      attachments,
      contexts: draft.contexts.filter(
        (context) =>
          context.upstream_model !== removed?.upstream_model ||
          attachments.some(
            (item) => item.upstream_model === context.upstream_model
          )
      ),
    }
  }
  if (draft.attachments.some((item) => item.route_id === route.id)) return draft
  return {
    ...draft,
    attachments: [
      ...draft.attachments,
      {
        route_id: route.id,
        upstream_model: route.model,
        priority: 0,
        weight: 1,
        speeds: [...route.speeds],
        timeout_ms: 30_000,
        payload: 'adapt',
      },
    ],
    contexts: draft.contexts.some((item) => item.upstream_model === route.model)
      ? draft.contexts
      : [...draft.contexts, newNativeContext(draft, route.model)],
  }
}
