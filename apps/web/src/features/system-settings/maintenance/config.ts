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
import {
  cloneHeaderNavDefaults,
  parseHeaderNavModules as parseHeaderNavConfig,
  parseHeaderNavBoolean,
  type HeaderNavModules as HeaderNavModulesConfig,
} from '@/lib/header-nav-config'

export type {
  HeaderNavModules as HeaderNavModulesConfig,
  ModuleAccess as HeaderNavAccessConfig,
} from '@/lib/header-nav-config'

export type SidebarSectionConfig = {
  enabled: boolean
  [key: string]: boolean
}

export type SidebarModulesAdminConfig = Record<string, SidebarSectionConfig>

export const HEADER_NAV_DEFAULT: HeaderNavModulesConfig =
  cloneHeaderNavDefaults()

export const SIDEBAR_MODULES_DEFAULT: SidebarModulesAdminConfig = {
  chat: {
    enabled: true,
    chat: true,
  },
  console: {
    enabled: true,
    detail: true,
    token: true,
    log: true,
    midjourney: true,
    task: true,
  },
  personal: {
    enabled: true,
    topup: true,
    personal: true,
  },
  admin: {
    enabled: true,
    channel: true,
    models: true,
    redemption: true,
    user: true,
    setting: true,
    subscription: true,
  },
}

const toBoolean = (value: unknown, fallback: boolean): boolean => {
  // Settings retain their existing rule: only the number 1 is true.
  return typeof value === 'number'
    ? value === 1
    : parseHeaderNavBoolean(value, fallback)
}

const cloneSidebarDefault = (): SidebarModulesAdminConfig =>
  Object.entries(SIDEBAR_MODULES_DEFAULT).reduce<SidebarModulesAdminConfig>(
    (acc, [section, config]) => {
      acc[section] = { ...config }
      return acc
    },
    {}
  )

export function parseHeaderNavModules(value: unknown): HeaderNavModulesConfig {
  return parseHeaderNavConfig(value)
}

export function serializeHeaderNavModules(
  config: HeaderNavModulesConfig
): string {
  return JSON.stringify(config)
}

export function parseSidebarModulesAdmin(
  value: string | null | undefined
): SidebarModulesAdminConfig {
  const defaults = cloneSidebarDefault()
  // If empty string, null, or undefined, use default config
  if (!value || value.trim() === '') return defaults

  try {
    const parsed = JSON.parse(value) as Record<string, unknown>
    const result: SidebarModulesAdminConfig = {}

    Object.entries(parsed).forEach(([sectionKey, raw]) => {
      if (!raw || typeof raw !== 'object') return

      const defaultSection = defaults[sectionKey] ?? { enabled: true }
      const sectionConfig: SidebarSectionConfig = {
        enabled: toBoolean(
          (raw as Record<string, unknown>).enabled,
          defaultSection.enabled ?? true
        ),
      }

      Object.entries(raw as Record<string, unknown>).forEach(
        ([moduleKey, moduleValue]) => {
          if (moduleKey === 'enabled') return
          if (sectionKey === 'chat' && moduleKey === 'playground') return
          sectionConfig[moduleKey] = toBoolean(
            moduleValue,
            defaultSection[moduleKey] ?? true
          )
        }
      )

      result[sectionKey] = sectionConfig
    })

    // Merge defaults to ensure expected sections exist
    Object.entries(defaults).forEach(([sectionKey, config]) => {
      if (!result[sectionKey]) {
        result[sectionKey] = { ...config }
        return
      }

      Object.entries(config).forEach(([moduleKey, moduleValue]) => {
        if (!(moduleKey in result[sectionKey])) {
          result[sectionKey][moduleKey] = moduleValue
        }
      })
    })

    return result
  } catch {
    return defaults
  }
}

export function serializeSidebarModulesAdmin(
  config: SidebarModulesAdminConfig
): string {
  return JSON.stringify(config)
}

export const toHeaderNavFormValues = (config: HeaderNavModulesConfig) => ({
  directory: config.directory ?? HEADER_NAV_DEFAULT.directory,
  developers: config.developers ?? HEADER_NAV_DEFAULT.developers,
  guide: config.guide ?? HEADER_NAV_DEFAULT.guide,
  scripts: config.scripts ?? HEADER_NAV_DEFAULT.scripts,
  webmcp: config.webmcp ?? HEADER_NAV_DEFAULT.webmcp,
  signal: config.signal ?? HEADER_NAV_DEFAULT.signal,
  challenges: config.challenges ?? HEADER_NAV_DEFAULT.challenges,
  home:
    config.home === undefined ? HEADER_NAV_DEFAULT.home : Boolean(config.home),
  console:
    config.console === undefined
      ? HEADER_NAV_DEFAULT.console
      : Boolean(config.console),
  pricingEnabled:
    config.pricing?.enabled === undefined
      ? HEADER_NAV_DEFAULT.pricing.enabled
      : Boolean(config.pricing.enabled),
  pricingRequireAuth:
    config.pricing?.requireAuth === undefined
      ? HEADER_NAV_DEFAULT.pricing.requireAuth
      : Boolean(config.pricing.requireAuth),
  rankingsEnabled:
    config.rankings?.enabled === undefined
      ? HEADER_NAV_DEFAULT.rankings.enabled
      : Boolean(config.rankings.enabled),
  rankingsRequireAuth:
    config.rankings?.requireAuth === undefined
      ? HEADER_NAV_DEFAULT.rankings.requireAuth
      : Boolean(config.rankings.requireAuth),
  securityEnabled:
    config.security?.enabled === undefined
      ? HEADER_NAV_DEFAULT.security.enabled
      : Boolean(config.security.enabled),
  securityRequireAuth:
    config.security?.requireAuth === undefined
      ? HEADER_NAV_DEFAULT.security.requireAuth
      : Boolean(config.security.requireAuth),
  docs:
    config.docs === undefined ? HEADER_NAV_DEFAULT.docs : Boolean(config.docs),
  about:
    config.about === undefined
      ? HEADER_NAV_DEFAULT.about
      : Boolean(config.about),
})

export type HeaderNavFormValues = ReturnType<typeof toHeaderNavFormValues>

export function applyHeaderNavFormValues(
  config: HeaderNavModulesConfig,
  values: HeaderNavFormValues
): HeaderNavModulesConfig {
  return {
    ...config,
    directory: values.directory,
    developers: values.developers,
    guide: values.guide,
    scripts: values.scripts,
    webmcp: values.webmcp,
    signal: values.signal,
    challenges: values.challenges,
    home: values.home,
    console: values.console,
    docs: values.docs,
    about: values.about,
    pricing: {
      ...(config.pricing ?? HEADER_NAV_DEFAULT.pricing),
      enabled: values.pricingEnabled,
      requireAuth: values.pricingRequireAuth,
    },
    rankings: {
      ...(config.rankings ?? HEADER_NAV_DEFAULT.rankings),
      enabled: values.rankingsEnabled,
      requireAuth: values.rankingsRequireAuth,
    },
    security: {
      ...(config.security ?? HEADER_NAV_DEFAULT.security),
      enabled: values.securityEnabled,
      requireAuth: values.securityRequireAuth,
    },
  }
}
