/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import type { AuthUser } from '@/stores/auth-store'

import { isConsoleActivated } from './console-activation'

/** API key management is available to every signed-in account. */
export function canManageApiKeys(user: AuthUser | null | undefined): boolean {
  return isConsoleActivated(user)
}
