/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

export const PROFILE_TABS = [
  'overview',
  'account',
  'security',
  'teams',
  'preferences',
  'rewards',
] as const
export type ProfileTab = (typeof PROFILE_TABS)[number]
export const profileSearchSchema = z.object({
  tab: z.enum(PROFILE_TABS).optional().catch(undefined),
})
