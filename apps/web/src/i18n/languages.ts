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
export const INTERFACE_LANGUAGE_OPTIONS = [
  { code: 'zh-CN', label: '简体中文' },
  { code: 'en', label: 'English' },
  { code: 'fr', label: 'Français' },
  { code: 'ru', label: 'Русский' },
  { code: 'ja', label: '日本語' },
  { code: 'vi', label: 'Tiếng Việt' },
  { code: 'zh-TW', label: '繁體中文' },
] as const

export type InterfaceLanguageCode =
  (typeof INTERFACE_LANGUAGE_OPTIONS)[number]['code']

export function normalizeInterfaceLanguage(
  value?: string | null
): InterfaceLanguageCode {
  if (!value) return 'en'

  const normalized = value.trim().replaceAll('_', '-').toLowerCase()
  if (
    normalized === 'zhtw' ||
    normalized === 'zh-tw' ||
    normalized === 'zh-hk' ||
    normalized === 'zh-mo' ||
    normalized.startsWith('zh-hant')
  ) {
    return 'zh-TW'
  }
  if (
    normalized === 'zh' ||
    normalized === 'zhcn' ||
    normalized === 'zh-cn' ||
    normalized === 'zh-sg' ||
    normalized.startsWith('zh-hans') ||
    normalized.startsWith('zh-')
  ) {
    return 'zh-CN'
  }

  return (
    INTERFACE_LANGUAGE_OPTIONS.find(
      (lang) => lang.code.toLowerCase() === normalized.split('-')[0]
    )?.code ?? 'en'
  )
}

/**
 * Map browser locales and legacy saved values to supported BCP 47 language tags.
 * Non-Chinese codes pass through for i18next's own supported-language matching.
 */
export function convertDetectedLanguage(value: string): string {
  const lower = value.trim().replaceAll('_', '-').toLowerCase()
  if (!lower.startsWith('zh')) return value
  if (
    lower === 'zhtw' ||
    lower === 'zh-tw' ||
    lower === 'zh-hk' ||
    lower === 'zh-mo' ||
    lower.startsWith('zh-hant')
  ) {
    return 'zh-TW'
  }
  return 'zh-CN'
}

/** Return a canonical BCP 47 tag for Intl APIs, or undefined when invalid. */
export function toIntlLocale(value?: string | null): string | undefined {
  if (!value) return undefined
  try {
    return Intl.getCanonicalLocales(value)[0]
  } catch {
    return undefined
  }
}
