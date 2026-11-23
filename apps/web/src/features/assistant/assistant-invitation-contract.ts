/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

import { AssistantStreamError } from './assistant-ai-stream'

export const INVITATION_PENDING = 'ASSISTANT_INVITATION_CONFIRMATION_PENDING'
const id = z.string().regex(/^[a-f0-9]{64}$/)
const integer = z.number().int().nonnegative().max(Number.MAX_SAFE_INTEGER)
const operation = z
  .string()
  .min(16)
  .max(128)
  .regex(/^\S+$/)
  .refine((value) =>
    Array.from(value).every((character) => {
      const point = character.charCodeAt(0)
      return point >= 0x20 && point !== 0x7f
    })
  )
const email = z
  .string()
  .email()
  .max(254)
  .regex(/^[\x21-\x7e]+$/)
const code = z.string().regex(/^(?:[A-Za-z0-9]{4}|[a-f0-9]{32})$/)
const actionSchema = z
  .object({
    type: z.literal('invitation_confirmation'),
    tool: z.literal('send_invitation'),
    confirmation_id: id,
    operation_id: operation,
    actor_user_id: integer.positive(),
    // Private response binding only. Never log, persist, or put this in chat text.
    actor_credential_sha256: id,
    expires_at: integer.positive(),
    recipient: email,
  })
  .strict()

const pendingSchema = z
  .object({
    success: z.literal(false),
    code: z.literal(INVITATION_PENDING),
    data: z
      .object({
        conversation_id: integer.positive(),
        client_turn_id: z.string().regex(/^[A-Za-z0-9][A-Za-z0-9_.:-]{0,63}$/),
        state: z.literal('tools_pending'),
        text: z.string().max(32768).optional(),
        gift: z.unknown().optional(),
      })
      .strict(),
    lmm_assistant_invitation: actionSchema,
    status: z.literal(202).optional(),
    message: z.string().max(512).optional(),
    retryable: z.literal(false).optional(),
  })
  .strict()

export type InvitationAction = z.infer<typeof actionSchema>
export type PendingInvitation = z.infer<typeof pendingSchema>
export function parsePendingInvitation(
  value: unknown,
  expected: {
    client_turn_id: string
    conversation_id?: number
    actor_user_id?: number
  }
): PendingInvitation | undefined {
  const parsed = pendingSchema.safeParse(value)
  if (!parsed.success) return
  const p = parsed.data
  if (
    p.data.client_turn_id !== expected.client_turn_id ||
    (expected.conversation_id !== undefined &&
      p.data.conversation_id !== expected.conversation_id) ||
    (expected.actor_user_id !== undefined &&
      p.lmm_assistant_invitation.actor_user_id !== expected.actor_user_id)
  ) {
    return
  }
  return p
}
export function pendingInvitationError(
  error: unknown,
  expected: {
    client_turn_id: string
    conversation_id?: number
    actor_user_id?: number
  }
): PendingInvitation | undefined {
  if (!(error instanceof AssistantStreamError) || error.status !== 202) return
  return parsePendingInvitation(error.response.data, expected)
}

const outcomeSchema = z
  .object({
    ok: z.boolean(),
    applied: z.boolean(),
    status: z.enum([
      'accepted_by_smtp',
      'rejected',
      'not_dispatched',
      'unknown',
    ]),
    smtp_accepted: z.boolean(),
    smtp_code: z.number().int().min(0).max(599).optional(),
    delivery_confirmed: z.literal(false),
    retry_safe: z.literal(false),
    operation_id: operation,
    confirmation_id: id,
    proof_receipt_id: z.string().min(1).max(256).optional(),
  })
  .strict()
  .refine((r) => {
    const accepted = r.status === 'accepted_by_smtp'
    if (
      r.ok !== accepted ||
      r.applied !== accepted ||
      r.smtp_accepted !== accepted
    ) {
      return false
    }
    if (accepted) return r.smtp_code === 250 && !!r.proof_receipt_id
    if (r.status === 'rejected') {
      return (
        !!r.proof_receipt_id && r.smtp_code !== undefined && r.smtp_code >= 400
      )
    }
    if (r.status === 'not_dispatched') {
      return !!r.proof_receipt_id && r.smtp_code === 0
    }
    return r.smtp_code === undefined || r.smtp_code === 0
  })
export type InvitationOutcome = z.infer<typeof outcomeSchema>
const viewSchema = z
  .object({
    id,
    operation_id: operation,
    tool: z.literal('send_invitation'),
    browser_url: z
      .string()
      .regex(/^\/api\/user\/auth\/assistant\/confirmations\/[a-f0-9]{64}$/),
    payload: z
      .object({
        kind: z.literal('invitation_email_v1'),
        email,
        invitation_code: code,
      })
      .strict(),
    expires_at: integer.positive(),
    consumed_at: integer.positive().nullable(),
    result: outcomeSchema.optional(),
    human_confirmed: z.boolean(),
    dispatch_started: z.boolean(),
  })
  .strict()

export type InvitationView = z.infer<typeof viewSchema>
export function parseInvitationOutcome(
  value: unknown,
  action: InvitationAction
): InvitationOutcome | undefined {
  const parsed = outcomeSchema.safeParse(value)
  if (
    !parsed.success ||
    parsed.data.operation_id !== action.operation_id ||
    parsed.data.confirmation_id !== action.confirmation_id
  ) {
    return
  }
  return parsed.data
}
export function parseInvitationView(
  value: unknown,
  action: InvitationAction
): InvitationView | undefined {
  const parsed = viewSchema.safeParse(value)
  if (!parsed.success) return
  const v = parsed.data
  if (
    v.id !== action.confirmation_id ||
    v.operation_id !== action.operation_id ||
    v.expires_at !== action.expires_at ||
    v.payload.email !== action.recipient
  ) {
    return
  }
  if ((v.consumed_at !== null) !== (v.result !== undefined)) return
  if (
    v.result &&
    (!v.dispatch_started || !parseInvitationOutcome(v.result, action))
  ) {
    return
  }
  if (v.human_confirmed && (v.dispatch_started || v.consumed_at !== null)) {
    return
  }
  return v
}
export function parseInvitationSuccess(value: unknown): unknown {
  const parsed = z
    .object({ success: z.literal(true), data: z.unknown() })
    .strict()
    .safeParse(value)
  return parsed.success ? parsed.data.data : undefined
}
