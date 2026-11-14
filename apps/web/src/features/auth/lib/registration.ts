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
import type { SystemStatus } from '../types'
import { federatedProviders, type FederatedProvider } from './federated-oauth'
import { isBuiltInGoogleConfigured } from './google-oauth'

type RegistrationSetting =
  | 'register_enabled'
  | 'password_register_enabled'
  | 'oauth_register_enabled'

function hasText(value: string | undefined): boolean {
  return Boolean(value?.trim())
}

/**
 * OAuth flags are not sufficient to launch a provider. The login handlers
 * intentionally return early when a required credential is missing, so do
 * not advertise those providers as clickable entry points in that state.
 */
export function isOAuthProviderConfigured(
  status: SystemStatus | null | undefined,
  method: string
): boolean {
  if (!status) return false

  const normalizedMethod = method.trim().toLowerCase()
  switch (normalizedMethod) {
    case 'google':
      return isBuiltInGoogleConfigured(status)
    case 'github':
      return (
        status.external_oauth_browser === true &&
        status.github_oauth === true &&
        hasText(status.github_client_id)
      )
    case 'discord':
      return (
        status.external_oauth_browser === true &&
        status.discord_oauth === true &&
        hasText(status.discord_client_id)
      )
    case 'linuxdo':
      return (
        status.external_oauth_browser === true &&
        status.linuxdo_oauth === true &&
        hasText(status.linuxdo_client_id)
      )
    case 'telegram':
      return status.telegram_oauth === true && hasText(status.telegram_bot_name)
    default:
      return false
  }
}

export function hasOAuthLoginProvider(
  status: SystemStatus | null | undefined
): boolean {
  return (
    federatedProviders(status).length > 0 ||
    ['google', 'github', 'discord', 'linuxdo', 'telegram'].some((method) =>
      isOAuthProviderConfigured(status, method)
    )
  )
}

function readRegistrationSetting(
  status: SystemStatus | null | undefined,
  key: RegistrationSetting
): boolean | undefined {
  const direct = status?.[key]
  if (typeof direct === 'boolean') return direct

  const nested = status?.data?.[key]
  return typeof nested === 'boolean' ? nested : undefined
}

export function isRegistrationEnabled(
  status: SystemStatus | null | undefined
): boolean {
  return readRegistrationSetting(status, 'register_enabled') !== false
}

export function isPasswordRegistrationEnabled(
  status: SystemStatus | null | undefined
): boolean {
  return (
    isRegistrationEnabled(status) &&
    readRegistrationSetting(status, 'password_register_enabled') !== false
  )
}

export function isOAuthRegistrationEnabled(
  status: SystemStatus | null | undefined
): boolean {
  return (
    isRegistrationEnabled(status) &&
    readRegistrationSetting(status, 'oauth_register_enabled') !== false
  )
}

export function getDisabledOAuthRegistrationMethods(
  status: SystemStatus | null | undefined
): ReadonlySet<string> {
  const methods =
    status?.oauth_registration_disabled_methods ??
    status?.data?.oauth_registration_disabled_methods ??
    []

  return new Set(
    methods
      .map((method) => method.trim().toLowerCase())
      .filter((method) => method.length > 0)
  )
}

export function isOAuthRegistrationAllowed(
  status: SystemStatus | null | undefined,
  provider: string | FederatedProvider
): boolean {
  if (!isOAuthRegistrationEnabled(status)) return false
  const disabled = getDisabledOAuthRegistrationMethods(status)
  if (typeof provider === 'string') {
    return !disabled.has(provider.trim().toLowerCase())
  }
  return (
    provider.registration_enabled === true &&
    !disabled.has(provider.kind) &&
    !disabled.has(`custom:${provider.slug}`) &&
    !disabled.has(`federated:${provider.id}`)
  )
}

/**
 * Returns whether at least one configured provider can create a new account.
 * Existing sign-in providers intentionally do not use this helper: the
 * registration policy must never hide or disable login.
 */
export function hasOAuthRegistrationProvider(
  status: SystemStatus | null | undefined
): boolean {
  if (!status || !isOAuthRegistrationEnabled(status)) return false

  if (
    federatedProviders(status).some((provider) =>
      isOAuthRegistrationAllowed(status, provider)
    )
  ) {
    return true
  }

  return ['google', 'github', 'discord', 'linuxdo', 'telegram'].some(
    (method) =>
      isOAuthProviderConfigured(status, method) &&
      isOAuthRegistrationAllowed(status, method)
  )
}

export function hasRegistrationMethod(
  status: SystemStatus | null | undefined
): boolean {
  return (
    isPasswordRegistrationEnabled(status) ||
    hasOAuthRegistrationProvider(status)
  )
}

/**
 * Registration links must wait for a live capability response. Cached status
 * is useful for layout, but it must not advertise a sign-up path that the
 * current server may have disabled.
 */
export function canOfferRegistration(
  status: SystemStatus | null | undefined,
  capabilitiesReady: boolean,
  localPreview = false
): boolean {
  return localPreview || (capabilitiesReady && hasRegistrationMethod(status))
}
