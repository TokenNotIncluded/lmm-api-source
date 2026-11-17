/*
Copyright (C) 2026 LIghtJUNction

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
*/
import type { AssistantPreConversationPreset } from './api'

// Bump both the query key and HTTP URL when reviewed starter copy changes.
export const ASSISTANT_PROMPT_PRESET_COPY_VERSION = 'model-help-v1'

type AssistantPresetViewer = {
  role: number
}

const requiredPresetKeys = {
  ai_recommendation: 'Which model should I use?',
  getting_started: 'Where should I start?',
  new_user_gift: 'How do I get the new-user gift?',
  weekly_discount: 'Any top-up discounts this week?',
} as const

const fallbackPresets: AssistantPreConversationPreset[] = Object.entries(
  requiredPresetKeys
).map(([id, prompt]) => ({ id, prompt, label: prompt }))

export function filterAssistantPreConversationPresets(
  presets: AssistantPreConversationPreset[] | undefined,
  _user: AssistantPresetViewer | null | undefined,
  capabilities?: { new_user_gift?: boolean }
): AssistantPreConversationPreset[] {
  return (presets ?? fallbackPresets).filter(
    (preset) =>
      (preset.id !== 'new_user_gift' || capabilities?.new_user_gift !== false)
  )
}

export function localizeAssistantPreConversationPresets(
  presets: AssistantPreConversationPreset[] | undefined,
  t: (key: string) => string
): AssistantPreConversationPreset[] {
  return (presets ?? fallbackPresets).map((preset) => {
    // A custom preset is localized by the server from the administrator's copy
    // for this language, so its text is already the right one. Unknown IDs are
    // never translated as keys.
    if (
      preset.source === 'custom' ||
      !Object.hasOwn(requiredPresetKeys, preset.id)
    ) {
      return preset
    }
    const key = requiredPresetKeys[preset.id as keyof typeof requiredPresetKeys]
    const prompt = t(key)
    return { ...preset, label: prompt, prompt }
  })
}
