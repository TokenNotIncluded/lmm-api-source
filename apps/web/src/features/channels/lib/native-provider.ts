/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { resolveModelProvider } from '@/lib/model-provider'

import type { NativeConfiguration, NativeProviderPreset } from '../native-api'

export function presetProtocolEndpoint(
  preset: NativeProviderPreset | undefined,
  oldProtocol: unknown,
  oldEndpoint: unknown,
  protocol: string
): string | undefined {
  if (!preset?.protocol_endpoints || typeof oldEndpoint !== 'string') return
  const variants = [preset, ...preset.protocol_endpoints]
  if (
    !variants.some(
      (item) =>
        item.protocol === oldProtocol &&
        item.endpoint === oldEndpoint.replace(/\/+$/, '')
    )
  ) {
    return
  }
  return variants.find((item) => item.protocol === protocol)?.endpoint
}

export function resolveNativeProvider(
  configuration: Pick<NativeConfiguration, 'provider_presets'>,
  provider: unknown,
  protocol: unknown,
  endpoint: unknown
) {
  const presets = configuration.provider_presets ?? []
  if (typeof provider === 'string' && provider) {
    return presets.find((preset) => preset.id === provider)
  }
  const base = typeof endpoint === 'string' ? endpoint.replace(/\/+$/, '') : ''
  const knownBase = presets.find(
    (preset) =>
      (preset.endpoint && preset.endpoint === base) ||
      preset.protocol_endpoints?.some(
        (item) => item.endpoint === base && item.protocol === protocol
      )
  )
  if (knownBase) return knownBase
  return presets.find((preset) => preset.id === 'custom')
}

type ProtocolSuggestion =
  | {
      source: 'operation' | 'model' | 'preset'
      protocol: string
      endpoint: string
    }
  | { source: 'ambiguous' | 'unknown' }

export function detectUpstreamApi(
  configuration: Pick<
    NativeConfiguration,
    'provider_presets' | 'routing' | 'bindings'
  >,
  draft: {
    endpoint: unknown
    provider?: unknown
    id?: unknown
    models: string[]
  }
): ProtocolSuggestion {
  if (typeof draft.endpoint !== 'string') return { source: 'unknown' }
  let url: URL
  try {
    url = new URL(draft.endpoint)
  } catch {
    return { source: 'unknown' }
  }
  if (
    url.protocol !== 'https:' ||
    url.username ||
    url.password ||
    url.search ||
    url.hash
  ) {
    return { source: 'unknown' }
  }
  const path = url.pathname.replace(/\/+$/, '')
  const endpoint = url.origin + path
  const operation = path.match(
    /^(.*\/v1(?:beta\d*)?)\/(messages|responses|chat\/completions|embeddings|rerank|models\/[^/]+:(?:streamGenerateContent|generateContent))$/
  )
  if (operation) {
    const protocols: Record<string, string> = {
      messages: 'anthropic_messages',
      responses: 'openai_responses',
      'chat/completions': 'openai_chat',
      embeddings: 'openai_embeddings',
      rerank: 'jina_rerank',
    }
    return {
      source: 'operation',
      protocol: protocols[operation[2]] ?? 'gemini',
      endpoint: url.origin + operation[1],
    }
  }
  const presets = configuration.provider_presets ?? []
  const matches = presets.flatMap((preset) =>
    [preset, ...(preset.protocol_endpoints ?? [])]
      .filter((item) => item.endpoint === endpoint)
      .map((item) => ({ provider: preset.id, protocol: item.protocol }))
  )
  // A published model target on this exact supplier and base can corroborate a mixed API.
  // Require every model in the draft to agree and exclude the draft itself.
  const supplier =
    typeof draft.provider === 'string' &&
    draft.provider &&
    draft.provider !== 'custom'
      ? draft.provider
      : matches[0]?.provider
  if (supplier && draft.models.length) {
    const protocolsByChannel = new Map(
      configuration.routing.upstreams
        .filter(
          (channel) =>
            channel.id !== draft.id &&
            channel.enabled &&
            channel.endpoint.replace(/\/+$/, '') === endpoint &&
            resolveNativeProvider(
              configuration,
              configuration.bindings.channel_providers?.[channel.id],
              channel.protocol,
              channel.endpoint
            )?.id === supplier
        )
        .map((channel) => [channel.id, channel.protocol])
    )
    const models = new Map(
      draft.models.map((model) => [model, new Set<string>()])
    )
    const groups = new Set(
      configuration.routing.groups
        .filter((group) => group.enabled)
        .map((group) => group.name)
    )
    for (const route of configuration.routing.routes) {
      if (!route.enabled || !groups.has(route.group)) continue
      for (const target of route.targets) {
        const protocol = protocolsByChannel.get(target.upstream_id)
        if (
          protocol &&
          target.enabled &&
          target.weight > 0 &&
          target.speeds.some((speed) => route.speeds.includes(speed))
        ) {
          models.get(target.upstream_model)?.add(protocol)
        }
      }
    }
    const sets = [...models.values()]
    const protocols = new Set(sets.flatMap((set) => [...set]))
    if (sets.every((set) => set.size === 1) && protocols.size === 1) {
      return { source: 'model', protocol: [...protocols][0], endpoint }
    }
    if (protocols.size > 1) return { source: 'ambiguous' }
  }
  if (!matches.length) return { source: 'unknown' }
  const protocols = new Set(matches.map((item) => item.protocol))
  if (
    protocols.size !== 1 ||
    matches.some((item) => item.provider === 'openrouter')
  )
    return { source: 'ambiguous' }
  return { source: 'preset', protocol: matches[0].protocol, endpoint }
}

const vendorIcons: Record<string, string> = {
  alibaba: 'Qwen',
  anthropic: 'Anthropic',
  deepseek: 'DeepSeek',
  google: 'Gemini',
  meta: 'Meta',
  mistral: 'Mistral',
  moonshot: 'Moonshot',
  openai: 'OpenAI',
  qwen: 'Qwen',
  xai: 'XAI',
  xiaomi: 'XiaomiMiMo',
  zai: 'ZAI',
  zhipu: 'Zhipu',
}

export function nativeModelLogo(
  configuration: NativeConfiguration,
  vendorName: string | undefined,
  model: string
): string | undefined {
  if (vendorName) {
    const name = vendorName.toLowerCase()
    const preset = configuration.provider_presets?.find(
      (item) => item.id === name || item.label.toLowerCase() === name
    )
    if (preset?.icon) return preset.icon
    const icon = vendorIcons[name.replaceAll(/[^a-z0-9]/g, '')]
    if (icon) return icon
  }
  return resolveModelProvider(model)?.icon
}
