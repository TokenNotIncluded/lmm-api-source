/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import axios from 'axios'
import { z } from 'zod'

import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

export type CommerceScope = { userId: number; sessionId: string }
export function commerceScope(): CommerceScope | undefined {
  const { user, session } = useAuthStore.getState().auth
  return user && Number.isSafeInteger(user.id) && user.id > 0 && session?.sid
    ? { userId: user.id, sessionId: session.sid }
    : undefined
}
export function currentCommerceScope(scope: CommerceScope) {
  const current = commerceScope()
  return (
    current?.userId === scope.userId && current.sessionId === scope.sessionId
  )
}
export class CommerceFailure extends Error {
  constructor(
    readonly code: string,
    readonly uncertain = true
  ) {
    super(code)
  }
}
export async function commerceRequest<T>(
  scope: CommerceScope,
  method: 'get' | 'post' | 'put',
  path: string,
  schema: z.ZodType<T>,
  data?: unknown
): Promise<T> {
  if (!currentCommerceScope(scope)) throw new CommerceFailure('SESSION_CHANGED')
  try {
    const response = await api.request({
      method,
      url: path,
      data,
      authScope: scope,
      disableDuplicate: true,
      skipAuthRefresh: true,
      skipBusinessError: true,
      skipErrorHandler: true,
      timeout: 15000,
    })
    if (!currentCommerceScope(scope)) {
      throw new CommerceFailure('SESSION_CHANGED')
    }
    const result = schema.safeParse(response.data)
    if (!result.success) throw new CommerceFailure('INVALID_RESPONSE')
    return result.data
  } catch (error) {
    if (!currentCommerceScope(scope)) {
      throw new CommerceFailure('SESSION_CHANGED')
    }
    if (error instanceof CommerceFailure) throw error
    const code = axios.isAxiosError(error)
      ? error.response?.data?.error?.code
      : undefined
    // Authorization is checked before receipt recovery. Losing permission on a
    // retry therefore does not prove that an earlier attempt failed to commit.
    const refused =
      typeof code === 'string' &&
      /^(SUBSCRIPTION|PROMOTION)_(INVALID|NOT_FOUND|CONFLICT|ALREADY_USED|BUDGET_EXHAUSTED)$/.test(
        code
      )
    throw new CommerceFailure(
      typeof code === 'string' ? code : 'RESULT_UNKNOWN',
      !refused
    )
  }
}
const exactUnitsPattern = /^(0|[1-9][0-9]{0,18})$/
export const exactUnits = z.string().regex(exactUnitsPattern)
export const positiveUnits = exactUnits.refine(
  (value) =>
    exactUnitsPattern.test(value) &&
    BigInt(value) > 0n &&
    BigInt(value) <= 1_000_000_000_000n
)
export const nativeId = z.number().int().positive().max(Number.MAX_SAFE_INTEGER)
export const nativeToken = z.string().regex(/^[a-f0-9]{64}$/)
export const operationKey = z.string().regex(/^[A-Za-z0-9_-]{16,80}$/)
export function requestKey() {
  return crypto.randomUUID()
}
function storageKey(scope: CommerceScope, area: string) {
  return `lmm-commerce:${scope.userId}:${scope.sessionId}:${area}`
}
export function readPending<T>(
  scope: CommerceScope,
  area: string,
  schema: z.ZodType<T>
): T | null {
  const raw = window.sessionStorage.getItem(storageKey(scope, area))
  return raw ? schema.parse(JSON.parse(raw)) : null
}
export function savePending(
  scope: CommerceScope,
  area: string,
  value: unknown | null
) {
  if (!currentCommerceScope(scope)) throw new CommerceFailure('SESSION_CHANGED')
  if (value === null) window.sessionStorage.removeItem(storageKey(scope, area))
  else {
    window.sessionStorage.setItem(
      storageKey(scope, area),
      JSON.stringify(value)
    )
  }
}
export function formatCredits(value: string, locale: string) {
  return BigInt(value).toLocaleString(locale)
}
