/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { resolveModelProvider } from '@/lib/model-provider'

import type { NativeConfiguration } from '../native-api'

export function resolveNativeProvider(
  configuration: NativeConfiguration,
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
    (preset) => preset.endpoint && preset.endpoint === base
  )
  if (knownBase) return knownBase
  const providerByProtocol: Record<string, string> = {
    anthropic_messages: 'anthropic',
    gemini: 'gemini',
    jina_rerank: 'jina',
    deepseek_search: 'deepseek',
    deepseek_fetch: 'deepseek',
    system_one: 'system_one',
  }
  const providerId =
    typeof protocol === 'string' ? providerByProtocol[protocol] : undefined
  return presets.find((preset) => preset.id === (providerId ?? 'custom'))
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
