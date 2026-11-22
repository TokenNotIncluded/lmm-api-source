/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { THEME_PRESETS } from '@/lib/theme-customization'

export const UI_PREFERENCE_OPTIONS = {
  mode: ['light', 'dark', 'system'],
  theme: THEME_PRESETS.map((preset) => preset.value),
  language: ['zh-CN', 'en', 'fr', 'ru', 'ja', 'vi', 'zh-TW'],
  currency: ['auto', 'USD', 'CNY', 'CREDIT'],
} as const

export type UIPreferences = {
  [
    K in keyof typeof UI_PREFERENCE_OPTIONS
  ]: (typeof UI_PREFERENCE_OPTIONS)[K][number]
}
export type UIPreferencePatch = Partial<UIPreferences>
type UIPreferenceActionFields = {
  type: 'workspace_action'
  tool: 'set_ui_preferences' | 'restore_ui_preferences'
  requires_confirmation: false
  confirmation_token?: never
  action_id: string
  actor_user_id: number
  expires_at: number
  temporary: boolean
  preview: UIPreferencePatch
}
export type AssistantUIPreferenceAction = UIPreferenceActionFields &
  (
    | { actor_session_id: string; actor_credential_sha256?: never }
    | { actor_credential_sha256: string; actor_session_id?: never }
  )

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

export function parseUIPreferencePatch(
  value: unknown
): UIPreferencePatch | undefined {
  if (!record(value)) return undefined
  const patch: Record<string, string> = {}
  for (const [key, selected] of Object.entries(value)) {
    if (!Object.hasOwn(UI_PREFERENCE_OPTIONS, key)) return undefined
    const values: readonly string[] =
      UI_PREFERENCE_OPTIONS[key as keyof UIPreferences]
    const normalized =
      key === 'theme' && selected === 'anthropic' ? 'ant' : selected
    if (typeof normalized !== 'string' || !values.includes(normalized)) {
      return undefined
    }
    patch[key] = normalized
  }
  return patch as UIPreferencePatch
}

// Parse data only. History inspection must never execute a browser action.
export function parseAssistantUIPreferenceAction(
  value: unknown
): AssistantUIPreferenceAction | undefined {
  if (!record(value)) return undefined
  const keys = [
    'type',
    'tool',
    'requires_confirmation',
    'action_id',
    'actor_user_id',
    'actor_session_id',
    'actor_credential_sha256',
    'expires_at',
    'temporary',
    'preview',
  ]
  if (
    Object.keys(value).length !== keys.length - 1 ||
    Object.keys(value).some((key) => !keys.includes(key))
  ) {
    return undefined
  }
  if (
    value.type !== 'workspace_action' ||
    (value.tool !== 'set_ui_preferences' &&
      value.tool !== 'restore_ui_preferences') ||
    value.requires_confirmation !== false ||
    typeof value.action_id !== 'string' ||
    !/^[a-f0-9]{32}$/.test(value.action_id) ||
    typeof value.actor_user_id !== 'number' ||
    !Number.isSafeInteger(value.actor_user_id) ||
    value.actor_user_id <= 0 ||
    typeof value.expires_at !== 'number' ||
    !Number.isSafeInteger(value.expires_at) ||
    value.expires_at <= 0 ||
    typeof value.temporary !== 'boolean'
  ) {
    return undefined
  }
  const legacySession =
    !Object.hasOwn(value, 'actor_credential_sha256') &&
    typeof value.actor_session_id === 'string' &&
    value.actor_session_id.length > 0 &&
    value.actor_session_id.length <= 256
  const nativeCredential =
    !Object.hasOwn(value, 'actor_session_id') &&
    typeof value.actor_credential_sha256 === 'string' &&
    /^[a-f0-9]{64}$/.test(value.actor_credential_sha256)
  if (!legacySession && !nativeCredential) return undefined
  const preview = parseUIPreferencePatch(value.preview)
  if (!preview) return undefined
  if (value.tool === 'restore_ui_preferences') {
    if (value.temporary || Object.keys(preview).length !== 0) return undefined
  } else {
    if (Object.keys(preview).length === 0) return undefined
    if (
      value.temporary &&
      (preview.language !== undefined || preview.currency !== undefined)
    ) {
      return undefined
    }
  }
  return { ...value, preview } as AssistantUIPreferenceAction
}

// Undo only our changed fields that still have the values we applied.
// A later manual choice must not be reset by an older assistant receipt.
export function uiPreferenceUndoPatch(
  before: UIPreferences,
  applied: UIPreferencePatch,
  current: UIPreferences
): UIPreferencePatch {
  const restore: Record<string, string> = {}
  for (const key of Object.keys(applied) as Array<keyof UIPreferences>) {
    if (current[key] === applied[key] && before[key] !== current[key]) {
      restore[key] = before[key]
    }
  }
  return restore as UIPreferencePatch
}
