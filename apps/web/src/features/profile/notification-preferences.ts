/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

import { api } from '@/lib/api'

const creditUnits = z
  .string()
  .regex(/^(0|[1-9][0-9]{0,15})$/)
  .refine(
    (value) =>
      /^(0|[1-9][0-9]{0,15})$/.test(value) && BigInt(value) <= 9007199254740991n
  )
export const notificationPreferencesSchema = z.object({
  enabled: z.boolean(),
  low_balance_units: creditUnits,
  revision: z.number().int().min(0).max(Number.MAX_SAFE_INTEGER),
  delivery: z.literal('in_app'),
})
export type NotificationPreferences = z.infer<
  typeof notificationPreferencesSchema
>
export type NotificationScope = {
  userId: number | undefined
  sessionId: string | undefined
}
type Envelope = { success: boolean; message?: string; data: unknown }

function unwrap(value: Envelope) {
  if (!value.success) {
    throw new Error(value.message || 'Unable to load notifications')
  }
  return notificationPreferencesSchema.parse(value.data)
}
export async function readNotificationPreferences(
  scope: NotificationScope,
  signal?: AbortSignal
) {
  const response = await api.get<Envelope>(
    '/core/v1/notifications/preferences',
    { authScope: scope, signal }
  )
  return unwrap(response.data)
}
export async function saveNotificationPreferences(
  scope: NotificationScope,
  input: NotificationPreferences
) {
  const value = notificationPreferencesSchema.parse(input)
  const body = {
    enabled: value.enabled,
    low_balance_units: value.low_balance_units,
    revision: value.revision,
  }
  const response = await api.put<Envelope>(
    '/core/v1/notifications/preferences',
    body,
    { authScope: scope }
  )
  return unwrap(response.data)
}
