/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import { ROLE } from './roles'

export const ASSISTANT_PRESET_SETTINGS_PATH =
  '/system-settings/content/assistant'

// The assistant publication has its own L5 read/L6 write API. Other settings
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
