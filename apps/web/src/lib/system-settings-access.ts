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
