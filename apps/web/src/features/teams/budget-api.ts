/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

import { request, type TeamScope } from './api'

const integerCredits = z.string().regex(/^(0|[1-9][0-9]*)$/)
const creditLimit = integerCredits.refine(
  (value) =>
    /^(0|[1-9][0-9]*)$/.test(value) &&
    value.length <= 19 &&
    BigInt(value) <= 9223372036854775807n
)
const budgetBaseSchema = z
  .object({
    id: z.string().regex(/^[1-9][0-9]*$/),
    scope: z.enum(['account', 'member', 'self']),
    account_id: z.number().int().safe().positive().nullable(),
    user_id: z.number().int().safe().positive().nullable(),
    issuer_user_id: z.number().int().safe().positive(),
    period: z.enum(['day', 'month']),
    revision: z.number().int().safe().positive(),
    limit_credits: creditLimit,
    consumed_credits: integerCredits,
    reserved_credits: integerCredits,
    window_start: z.number().int().safe(),
    window_end: z.number().int().safe(),
    can_edit: z.boolean(),
  })
  .strict()
export const budgetSchema = z.discriminatedUnion('unlimited', [
  budgetBaseSchema.extend({
    unlimited: z.literal(false),
    remaining_credits: creditLimit,
  }),
  budgetBaseSchema.extend({
    unlimited: z.literal(true),
    remaining_credits: z.null(),
  }),
])
const budgetListSchema = z
  .object({
    version: z.literal(1),
    items: z.array(budgetSchema),
  })
  .strict()
export type Budget = z.infer<typeof budgetSchema>
export type BudgetDraft = {
  scope: Budget['scope']
  user_id?: number
  period: Budget['period']
  limit_credits: string
  expected_revision: number | null
}
const teamIdSchema = z.number().int().safe().positive()
const path = (teamId?: number) =>
  teamId === undefined
    ? '/core/v1/budgets'
    : `/core/v1/teams/${teamIdSchema.parse(teamId)}/budgets`

export const listBudgets = (
  scope: TeamScope,
  teamId?: number,
  signal?: AbortSignal
) =>
  request(
    scope,
    'GET',
    path(teamId),
    budgetListSchema.refine((page) =>
      page.items.every((item) =>
        teamId === undefined
          ? item.scope === 'self' && item.user_id === scope.userId
          : item.scope !== 'self'
      )
    ),
    undefined,
    signal
  )

export const saveBudget = (
  scope: TeamScope,
  draft: BudgetDraft,
  teamId?: number
) => {
  const limit_credits = creditLimit.parse(draft.limit_credits.trim())
  const period = z.enum(['day', 'month']).parse(draft.period)
  const expected_revision = z
    .number()
    .int()
    .safe()
    .positive()
    .nullable()
    .parse(draft.expected_revision)
  if (teamId === undefined && draft.scope !== 'self') {
    throw new Error('invalid_budget_scope')
  }
  if (teamId !== undefined && draft.scope === 'self') {
    throw new Error('invalid_budget_scope')
  }
  const data =
    teamId === undefined
      ? { period, limit_credits, expected_revision }
      : {
          scope: draft.scope,
          ...(draft.scope === 'member'
            ? { user_id: teamIdSchema.parse(draft.user_id) }
            : {}),
          period,
          limit_credits,
          expected_revision,
        }
  return request(
    scope,
    'PUT',
    path(teamId),
    budgetSchema.refine(
      (item) =>
        !item.unlimited &&
        item.scope === draft.scope &&
        item.period === period &&
        item.limit_credits === limit_credits &&
        item.revision === (expected_revision ?? 0) + 1 &&
        item.issuer_user_id === scope.userId &&
        item.user_id ===
          (draft.scope === 'self'
            ? scope.userId
            : draft.scope === 'member'
              ? draft.user_id
              : null)
    ),
    data
  )
}

export function validCreditLimit(value: string): boolean {
  return creditLimit.safeParse(value.trim()).success
}

export function budgetProgress(budget: Budget): number {
  if (budget.unlimited) return 0
  const limit = BigInt(budget.limit_credits)
  if (limit === 0n) return 100
  const allocated =
    BigInt(budget.consumed_credits) + BigInt(budget.reserved_credits)
  return Math.min(100, Number((allocated * 10000n) / limit) / 100)
}
