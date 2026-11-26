/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.

For commercial licensing, please contact support@quantumnous.com
*/
/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
interface ModelProvider {
  icon?: string
  label: string
}

export function resolveModelProvider(modelName: string): ModelProvider | null {
  const model = modelName.toLowerCase()
  const hasAny = (keywords: string[]) =>
    keywords.some((keyword) => model.includes(keyword))

  if (
    hasAny([
      'gpt-',
      'chatgpt-',
      'text-embedding-',
      'omni-moderation',
      'dall-e',
      'whisper',
      'tts-',
    ]) ||
    /\bo[134](?:-|$)/.test(model)
  ) {
    return { icon: 'OpenAI.Color', label: 'OpenAI' }
  }
  if (hasAny(['claude-', 'anthropic'])) {
    return { icon: 'Claude.Color', label: 'Anthropic' }
  }
  if (hasAny(['gemini-', 'gemma-', 'learnlm-'])) {
    return { icon: 'Gemini.Color', label: 'Google' }
  }
  if (hasAny(['grok-', 'xai-'])) {
    return { icon: 'Grok.Color', label: 'xAI' }
  }
  if (hasAny(['deepseek-'])) {
    return { icon: 'DeepSeek.Color', label: 'DeepSeek' }
  }
  if (hasAny(['qwen', 'qwq-'])) {
    return { icon: 'Qwen.Color', label: 'Qwen' }
  }
  if (hasAny(['doubao-', 'volcengine'])) {
    return { icon: 'Doubao.Color', label: 'Doubao' }
  }
  if (hasAny(['moonshot-', 'kimi-'])) {
    return { icon: 'Moonshot.Color', label: 'Moonshot' }
  }
  if (hasAny(['minimax', 'abab'])) {
    return { icon: 'Minimax.Color', label: 'MiniMax' }
  }
  if (hasAny(['glm-', 'chatglm', 'cogview', 'cogvideo'])) {
    return { icon: 'Zhipu.Color', label: 'Zhipu' }
  }
  if (hasAny(['mimo-'])) {
    return { icon: 'XiaomiMiMo', label: 'Xiaomi' }
  }
  if (hasAny(['ernie'])) {
    return { icon: 'Wenxin.Color', label: 'Baidu' }
  }
  if (hasAny(['spark']) && !hasAny(['muse-spark-'])) {
    return { icon: 'Spark.Color', label: 'iFlyTek' }
  }
  if (hasAny(['hunyuan'])) {
    return { icon: 'Hunyuan.Color', label: 'Tencent' }
  }
  if (hasAny(['baichuan'])) {
    return { icon: 'Baichuan.Color', label: 'Baichuan' }
  }
  if (hasAny(['internlm'])) {
    return { icon: 'InternLM.Color', label: 'InternLM' }
  }
  if (hasAny(['step-'])) {
    return { icon: 'Stepfun.Color', label: 'StepFun' }
  }
  if (hasAny(['yi-'])) {
    return { icon: 'Yi.Color', label: 'Yi' }
  }
  if (hasAny(['mistral-', 'mixtral-'])) {
    return { icon: 'Mistral.Color', label: 'Mistral' }
  }
  if (hasAny(['llama-', 'meta-', 'muse-spark-'])) {
    return { icon: 'Meta.Color', label: 'Meta' }
  }
  if (hasAny(['command-', 'cohere-', 'cohere/'])) {
    return { icon: 'Cohere.Color', label: 'Cohere' }
  }

  if (model.startsWith('dots-studio/')) {
    return { icon: 'DotsStudio', label: 'Dots Studio' }
  }
  if (model.startsWith('recursive/')) return { label: 'Recursive' }
  if (model === 'jev-latest') return { label: 'Jev' }
  if (model === 'openrouter/free')
    return { icon: 'OpenRouter', label: 'OpenRouter' }
  return null
}
