/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import { CanceledError } from 'axios'

import { api, type ApiRequestConfig } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

export type AssistantPresetSettings = {
  configuration: string
  revision: number
  updated_at: number
}

const SETTINGS_PATH = '/api/assistant/settings/pre-conversation-presets'

function record(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
}

export function validPresetConfiguration(configuration: string): boolean {
  if ([...configuration].length > 48000) return false
  if (!configuration.trim()) return true
  try {
    const presets: unknown = JSON.parse(configuration)
    if (!Array.isArray(presets) || presets.length > 20) return false
    const ids = new Set<string>()
    const copy = (value: unknown, max: number): boolean => {
      const values = typeof value === 'string' ? { default: value } : value
      return (
        record(values) &&
        Object.keys(values).length <= 16 &&
        typeof values.default === 'string' &&
        Object.entries(values).every(
          ([locale, text]) =>
            /^[A-Za-z0-9_-]{1,32}$/.test(locale) &&
            typeof text === 'string' &&
            [...text.trim()].length > 0 &&
            [...text.trim()].length <= max &&
            !text.includes('\0')
        )
      )
    }
    return presets.every((preset: unknown) => {
      if (
        !record(preset) ||
        Object.keys(preset).length !== 3 ||
        typeof preset.id !== 'string' ||
        !/^[A-Za-z0-9_-]{1,64}$/.test(preset.id) ||
        ids.has(preset.id) ||
        !copy(preset.label, 80) ||
        !copy(preset.prompt, 2000)
      ) {
        return false
      }
      ids.add(preset.id)
      return true
    })
  } catch {
    return false
  }
}

function settingsEnvelope(envelope: unknown): AssistantPresetSettings {
  if (
    !record(envelope) ||
    envelope.success !== true ||
    !record(envelope.data)
  ) {
    throw new Error('Invalid assistant preset settings response')
  }
  const value = envelope.data
  if (
    typeof value.configuration !== 'string' ||
    !validPresetConfiguration(value.configuration) ||
    typeof value.revision !== 'number' ||
    !Number.isSafeInteger(value.revision) ||
    value.revision < 0 ||
    typeof value.updated_at !== 'number' ||
    !Number.isSafeInteger(value.updated_at) ||
    value.updated_at < 0 ||
    (value.revision === 0
      ? value.updated_at !== 0 || value.configuration !== ''
      : value.updated_at === 0)
  ) {
    throw new Error('Invalid assistant preset settings response')
  }
  return {
    configuration: value.configuration,
    revision: value.revision,
    updated_at: value.updated_at,
  }
}

function requireSameSession(options: ApiRequestConfig) {
  const current = useAuthStore.getState().auth
  if (
    !options.authScope ||
    options.authScope.userId !== current.user?.id ||
    options.authScope.sessionId !== current.session?.sid
  ) {
    throw new CanceledError(
      'Authentication changed during preset settings request'
    )
  }
}

export async function getAssistantPresetSettings(options: ApiRequestConfig) {
  requireSameSession(options)
  const response = await api.get<unknown>(SETTINGS_PATH, {
    ...options,
    skipAuthRefresh: true,
  })
  requireSameSession(options)
  return settingsEnvelope(response.data)
}

export async function saveAssistantPresetSettings(
  value: Pick<AssistantPresetSettings, 'configuration' | 'revision'>,
  options: ApiRequestConfig
) {
  requireSameSession(options)
  const response = await api.put<unknown>(SETTINGS_PATH, value, {
    ...options,
    skipAuthRefresh: true,
    skipBusinessError: true,
    skipErrorHandler: true,
  })
  requireSameSession(options)
  return settingsEnvelope(response.data)
}

export function presetSettingsConflict(error: unknown): boolean {
  return (
    record(error) && record(error.response) && error.response.status === 409
  )
}
