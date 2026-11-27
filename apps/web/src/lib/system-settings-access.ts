/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import { ROLE } from './roles'

export const ASSISTANT_PRESET_SETTINGS_PATH =
  '/system-settings/content/assistant'

// These settings support administrator reads and owner writes.
const adminReadablePaths = new Set([
  ASSISTANT_PRESET_SETTINGS_PATH,
  '/system-settings/security/market-ai-review',
])
export function canReadSystemSettingsPath(
  role: number | undefined,
  pathname: string
): boolean {
  return (
    role === ROLE.SUPER_ADMIN ||
    (role === ROLE.ADMIN &&
      adminReadablePaths.has(pathname.replace(/\/+$/, '')))
  )
}

export const CORE_SETTINGS_HOME = '/system-settings'
export const EXTENSION_SETTINGS_HOME = '/extensions/settings'

const corePaths = new Set([
  '/system-settings/auth/basic-auth',
  '/system-settings/auth/passkey',
  '/system-settings/billing/quota',
  '/system-settings/billing/currency',
  '/system-settings/billing/model-pricing',
  '/system-settings/billing/router-pricing',
  '/system-settings/billing/group-pricing',
  CORE_SETTINGS_HOME,
  '/users',
  '/keys',
  '/system-settings/models/global',
  '/system-settings/models/routing-reliability',
  '/system-settings/models/channel-affinity',
  '/system-settings/security/ip-access-routing',
  '/system-settings/security/rate-limit',
  '/system-settings/security/token-limits',
])

export type SettingsScope = 'core' | 'extensions'

export function getSettingsScope(url: string): SettingsScope {
  const pathname = url.split('?')[0].replace(/\/+$/, '')
  return corePaths.has(pathname) ? 'core' : 'extensions'
}

export function getSettingsHomeForPath(url: string) {
  const pathname = url.split(/[?#]/)[0].replace(/\/+$/, '')
  if (
    pathname === EXTENSION_SETTINGS_HOME ||
    pathname.startsWith(`${EXTENSION_SETTINGS_HOME}/`)
  ) {
    return EXTENSION_SETTINGS_HOME
  }
  if (
    pathname === CORE_SETTINGS_HOME ||
    pathname.startsWith(`${CORE_SETTINGS_HOME}/`)
  ) {
    return getSettingsScope(pathname) === 'core'
      ? CORE_SETTINGS_HOME
      : EXTENSION_SETTINGS_HOME
  }
  return null
}

export function getSettingsTitle(scope: SettingsScope) {
  return scope === 'core' ? 'Core Settings' : 'Plugin Settings'
}
