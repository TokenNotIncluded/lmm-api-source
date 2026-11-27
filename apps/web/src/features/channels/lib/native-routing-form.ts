/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import type { TFunction } from 'i18next'
import { z } from 'zod'

import {
  parseNativeDocument,
  type NativeContext,
  type NativeConfiguration,
} from '../native-api'
import { newNativeTargetTimeoutMs } from './native-target-timeout'

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

export function nativeProtocolLabel(protocol: string, t: TFunction): string {
  switch (protocol) {
    case 'openai_chat':
      return t('OpenAI Chat Completions')
    case 'openai_responses':
      return t('OpenAI Responses')
    case 'anthropic_messages':
      return t('Anthropic Messages')
    case 'gemini':
      return t('Gemini')
    case 'system_one':
      return t('System One')
    case 'openai_embeddings':
      return t('OpenAI Embeddings')
    case 'jina_rerank':
      return t('Jina Rerank')
    case 'deepseek_search':
      return t('DeepSeek Search')
    case 'deepseek_fetch':
      return t('DeepSeek Fetch')
    default:
      return protocol
  }
}

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

export function readChannelDraft(document: string): NativeChannelDraft | null {
  try {
    return channelDraftSchema.parse(parseNativeDocument(document))
  } catch {
    return null
  }
}

export function newNativeContext(
  upstream: { id: number; endpoint: string; protocol: string },
  model: string,
  publishedContexts: readonly NativeContext[] = []
): NativeContext {
  const published = publishedContexts.find(
    (context) =>
      context.upstream_id === upstream.id && context.upstream_model === model
  )
  if (published) {
    return {
      ...published,
      endpoint: upstream.endpoint,
      protocol: upstream.protocol,
    }
  }
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
      contexts = [
        ...contexts,
        newNativeContext(draft, patch.upstream_model, publishedContexts),
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
  enabled: boolean,
  configuration?: Pick<NativeConfiguration, 'model_limits'> & {
    bindings?: Pick<NativeConfiguration['bindings'], 'contexts'>
  }
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
        timeout_ms: newNativeTargetTimeoutMs(configuration),
        payload: 'adapt',
      },
    ],
    contexts: draft.contexts.some((item) => item.upstream_model === route.model)
      ? draft.contexts
      : [
          ...draft.contexts,
          newNativeContext(
            draft,
            route.model,
            configuration?.bindings?.contexts
          ),
        ],
  }
}
