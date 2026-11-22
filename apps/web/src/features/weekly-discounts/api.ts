/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

export const weeklyAPIPath = '/api/user/auth/assistant/weekly-discounts'
export const weeklyDecisionID = /^[0-9a-f]{64}$/
const stamp = z
  .number()
  .int()
  .nonnegative()
  .max(8_640_000_000_000 - 604_800)
const utcWeek = stamp.refine((value) => (value - 345_600) % 604_800 === 0)
const decisionFields = {
  id: z.string().regex(weeklyDecisionID),
  week_start: utcWeek,
  expires_at: stamp,
  status: z.enum(['offered', 'claimed', 'declined']),
  discount_percent: z.number().int().min(0).max(99),
  created_at: stamp,
  claimed_at: stamp.nullable(),
  browser_path: z.string(),
}
const decisionBaseSchema = z.object(decisionFields)
function boundDecision(
  value: z.infer<typeof decisionBaseSchema>,
  context: z.RefinementCtx
) {
  if (
    value.browser_path !== weeklyAPIPath + '/' + value.id ||
    value.expires_at !== value.week_start + 604_800 ||
    (value.status === 'declined') !== (value.discount_percent === 0) ||
    (value.status === 'claimed') !== (value.claimed_at !== null)
  ) {
    context.addIssue({
      code: 'custom',
      message: 'Invalid weekly decision binding',
    })
  }
}
const summarySchema = decisionBaseSchema.strict().superRefine(boundDecision)
const detailSchema = z
  .object({
    ...decisionFields,
    reason: z.string().max(4096),
    code: z.string().min(1).max(1024).optional(),
  })
  .strict()
  .superRefine(boundDecision)
  .superRefine((value, context) => {
    if ((value.status === 'claimed') !== (value.code !== undefined)) {
      context.addIssue({
        code: 'custom',
        message: 'Invalid private code projection',
      })
    }
  })
const historySchema = z
  .object({
    ok: z.literal(true),
    current_utc_week: utcWeek,
    decisions: z.array(summarySchema).max(100),
    next_before_week: utcWeek.nullable(),
  })
  .strict()
  .superRefine((value, context) => {
    const seen = new Set<string>()
    let previousWeek = Number.POSITIVE_INFINITY
    for (const decision of value.decisions) {
      if (
        seen.has(decision.id) ||
        decision.week_start >= previousWeek ||
        decision.week_start > value.current_utc_week
      ) {
        context.addIssue({
          code: 'custom',
          message: 'Invalid weekly history order',
        })
      }
      seen.add(decision.id)
      previousWeek = decision.week_start
    }
    if (
      value.next_before_week !== null &&
      (value.decisions.length !== 100 ||
        value.next_before_week !== value.decisions.at(-1)?.week_start)
    ) {
      context.addIssue({
        code: 'custom',
        message: 'Invalid weekly history cursor',
      })
    }
  })
const inspectionSchema = z
  .object({
    ok: z.literal(true),
    decision: detailSchema,
    claimable: z.boolean(),
    csrf: z.string().regex(weeklyDecisionID).optional(),
  })
  .strict()
  .superRefine((value, context) => {
    if (
      value.claimable &&
      (value.decision.status !== 'offered' || value.csrf === undefined)
    ) {
      context.addIssue({
        code: 'custom',
        message: 'Missing browser claim authority',
      })
    }
  })
const claimedSchema = z
  .object({
    ok: z.literal(true),
    decision: detailSchema,
    executed: z.literal(true),
  })
  .strict()
  .refine((value) => value.decision.status === 'claimed')

export type WeeklyDecisionSummary = z.infer<typeof summarySchema>
export type WeeklyDecisionDetail = z.infer<typeof detailSchema>
export type WeeklyHistory = z.infer<typeof historySchema>
export type WeeklyInspection = z.infer<typeof inspectionSchema>

export class WeeklyRequestFailure extends Error {
  constructor() {
    // Response bodies may contain a private coupon. Never include them in errors.
    super('The weekly discount response could not be verified.')
    this.name = 'WeeklyRequestFailure'
  }
}
function parse<T>(schema: z.ZodType<T>, value: unknown): T {
  const parsed = schema.safeParse(value)
  if (!parsed.success) {
    throw new WeeklyRequestFailure()
  }
  return parsed.data
}
export function parseWeeklyHistory(value: unknown): WeeklyHistory {
  return parse(historySchema, value)
}
export function parseWeeklyInspection(
  value: unknown,
  id: string
): WeeklyInspection {
  const result = parse(inspectionSchema, value)
  if (!weeklyDecisionID.test(id) || result.decision.id !== id) {
    throw new WeeklyRequestFailure()
  }
  return result
}
export function parseWeeklyClaim(
  value: unknown,
  original: WeeklyDecisionDetail
) {
  const result = parse(claimedSchema, value)
  const next = result.decision
  if (
    next.id !== original.id ||
    next.week_start !== original.week_start ||
    next.expires_at !== original.expires_at ||
    next.discount_percent !== original.discount_percent ||
    next.created_at !== original.created_at ||
    next.reason !== original.reason ||
    next.browser_path !== original.browser_path
  ) {
    throw new WeeklyRequestFailure()
  }
  return result
}

async function readJSON(response: Response): Promise<unknown> {
  if (
    !response.ok ||
    response.headers
      .get('content-type')
      ?.split(';', 1)[0]
      .trim()
      .toLowerCase() !== 'application/json'
  ) {
    throw new WeeklyRequestFailure()
  }
  const reader = response.body?.getReader()
  if (!reader) {
    throw new WeeklyRequestFailure()
  }
  let size = 0
  const parts: Uint8Array[] = []
  try {
    for (;;) {
      const { done, value } = await reader.read()
      if (done) {
        break
      }
      size += value.byteLength
      if (size > 256 * 1024) {
        await reader.cancel()
        throw new WeeklyRequestFailure()
      }
      parts.push(value)
    }
    const bytes = new Uint8Array(size)
    let offset = 0
    for (const part of parts) {
      bytes.set(part, offset)
      offset += part.byteLength
    }
    return JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes))
  } catch {
    throw new WeeklyRequestFailure()
  } finally {
    reader.releaseLock()
  }
}

export function createWeeklyClient(fetcher: typeof fetch = globalThis.fetch) {
  async function request(
    path: string,
    signal: AbortSignal,
    csrf?: string
  ): Promise<unknown> {
    const headers: Record<string, string> = { Accept: 'application/json' }
    if (csrf !== undefined) {
      if (!weeklyDecisionID.test(csrf)) {
        throw new WeeklyRequestFailure()
      }
      headers['Content-Type'] = 'application/json'
      headers['x-lmm-assistant-csrf'] = csrf
    }
    try {
      return await readJSON(
        await fetcher(path, {
          method: csrf === undefined ? 'GET' : 'POST',
          headers,
          credentials: 'same-origin',
          mode: 'same-origin',
          cache: 'no-store',
          redirect: 'error',
          referrerPolicy: 'no-referrer',
          signal,
          ...(csrf === undefined ? {} : { body: '{"claim":true}' }),
        })
      )
    } catch {
      throw new WeeklyRequestFailure()
    }
  }
  function pathFor(id: string) {
    if (!weeklyDecisionID.test(id)) {
      throw new WeeklyRequestFailure()
    }
    return weeklyAPIPath + '/' + id
  }
  return {
    async history(signal: AbortSignal, beforeWeek?: number) {
      if (beforeWeek !== undefined && !utcWeek.safeParse(beforeWeek).success) {
        throw new WeeklyRequestFailure()
      }
      const result = parseWeeklyHistory(
        await request(
          weeklyAPIPath +
            (beforeWeek === undefined
              ? ''
              : '?before_week=' + String(beforeWeek)),
          signal
        )
      )
      if (
        beforeWeek !== undefined &&
        result.decisions.some((decision) => decision.week_start >= beforeWeek)
      ) {
        throw new WeeklyRequestFailure()
      }
      return result
    },
    async inspect(id: string, signal: AbortSignal) {
      return parseWeeklyInspection(await request(pathFor(id), signal), id)
    },
    async claim(
      original: WeeklyDecisionDetail,
      csrf: string,
      signal: AbortSignal
    ) {
      return parseWeeklyClaim(
        await request(pathFor(original.id), signal, csrf),
        original
      )
    },
  }
}
export type WeeklyClient = ReturnType<typeof createWeeklyClient>
