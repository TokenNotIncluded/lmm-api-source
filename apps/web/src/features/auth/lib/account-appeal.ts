/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

export const appealRequestSchema = z.object({
  id: z.number(),
  status: z.enum(['pending', 'approved', 'rejected', 'superseded']),
  reason: z.string(),
  created_at: z.number(),
  reviewed_at: z.number().nullable(),
})
export const appealContextSchema = z.object({
  username: z.string(),
  public_reason: z.string().nullable(),
  public_reason_available: z.boolean(),
  expires_at: z.number(),
  request: appealRequestSchema.nullable(),
})
export const appealReceiptSchema = appealRequestSchema.extend({
  already_pending: z.boolean(),
})
export type AccountAppealContext = z.infer<typeof appealContextSchema>
export type AccountAppealReceipt = z.infer<typeof appealReceiptSchema>

const disabledAppealSchema = z.object({
  code: z.literal('AUTH_USER_DISABLED'),
  data: appealContextSchema.omit({ request: true }).extend({
    appeal_available: z.literal(true),
  }),
})

/** Only an explicit disabled response can offer verified-identity appeal. */
export function hasVerifiedAccountAppeal(value: unknown): boolean {
  const httpError = z
    .object({
      response: z.object({ status: z.literal(403), data: z.unknown() }),
    })
    .safeParse(value)
  return disabledAppealSchema.safeParse(
    httpError.success ? httpError.data.response.data : value
  ).success
}

const APPEAL_HINT_KEY = 'lmm.verified-account-appeal'
let verifiedAppealHint = false

/** A tab-local UI hint; the server's HttpOnly Cookie remains the only proof. */
export function hasVerifiedAccountAppealHint(): boolean {
  try {
    return (
      verifiedAppealHint ||
      window.sessionStorage.getItem(APPEAL_HINT_KEY) === '1'
    )
  } catch {
    return verifiedAppealHint
  }
}

export function setVerifiedAccountAppealHint(available: boolean) {
  verifiedAppealHint = available
  try {
    if (available) window.sessionStorage.setItem(APPEAL_HINT_KEY, '1')
    else window.sessionStorage.removeItem(APPEAL_HINT_KEY)
  } catch {
    // Keep the current navigation usable when browser storage is unavailable.
  }
}

export const appealStatusKeys = {
  pending: 'Pending review',
  approved: 'Approved',
  rejected: 'Rejected',
  superseded: 'Superseded',
} as const
