/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import { ROLE } from './roles'

export const ASSISTANT_PRESET_SETTINGS_PATH =
  '/system-settings/content/assistant'

// Assistant publication allows administrator reads and super-administrator writes. Other settings
// retain their existing super-administrator route and navigation restriction.
export function canReadSystemSettingsPath(
  role: number | undefined,
  pathname: string
): boolean {
  return (
    role === ROLE.SUPER_ADMIN ||
    (role === ROLE.ADMIN &&
      pathname.replace(/\/+$/, '') === ASSISTANT_PRESET_SETTINGS_PATH)
  )
}

export const CORE_SETTINGS_HOME = '/system-settings'
export const EXTENSION_SETTINGS_HOME = '/system-settings/site'

const corePaths = new Set([
  '/system-settings/auth/basic-auth',
  '/system-settings/auth/passkey',
  '/system-settings/billing/quota',
  '/system-settings/billing/currency',
  '/system-settings/billing/model-pricing',
  '/system-settings/billing/router-pricing',
  '/system-settings/billing/group-pricing',
  CORE_SETTINGS_HOME,
  '/system-settings/billing/payment',
  '/system-settings/models/global',
  '/system-settings/models/service-tier-pricing',
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

export function getSettingsTitle(scope: SettingsScope) {
  return scope === 'core' ? 'Core Settings' : 'Extension Settings'
}
