/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import axios from 'axios'
import { z } from 'zod'

import { api, withAuthenticatedCookieWrite } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

export type IPAllowlistScope = { userId: number; sessionId: string }
export type IPAllowlist = { allow_ips: string[]; client_ip: string | null }

const responseSchema = z.object({
  success: z.literal(true),
  data: z.object({
    allow_ips: z.array(z.string()).max(128),
    client_ip: z.string().nullable(),
  }),
})

export class IPAllowlistFailure extends Error {
  constructor(readonly code: string) {
    super(code)
    this.name = 'IPAllowlistFailure'
  }
}

export function isCurrentIPAllowlistScope(scope: IPAllowlistScope) {
  const auth = useAuthStore.getState().auth
  return auth.user?.id === scope.userId && auth.session?.sid === scope.sessionId
}

function assertScope(scope: IPAllowlistScope) {
  if (!isCurrentIPAllowlistScope(scope)) {
    throw new IPAllowlistFailure('SESSION_CHANGED')
  }
}

function readResponse(value: unknown): IPAllowlist {
  const parsed = responseSchema.safeParse(value)
  if (parsed.success) return parsed.data.data
  const failure = z.object({ code: z.string() }).safeParse(value)
  throw new IPAllowlistFailure(
    failure.success ? failure.data.code : 'INVALID_RESPONSE'
  )
}

function responseFailure(error: unknown): never {
  if (axios.isAxiosError(error)) {
    const failure = z
      .object({ code: z.string() })
      .safeParse(error.response?.data)
    if (failure.success) throw new IPAllowlistFailure(failure.data.code)
  }
  throw error
}

export async function getIPAllowlist(
  scope: IPAllowlistScope,
  signal?: AbortSignal
): Promise<IPAllowlist> {
  assertScope(scope)
  try {
    const response = await api.get('/api/user/self/ip-allowlist', {
      signal,
      authScope: scope,
      skipBusinessError: true,
      skipErrorHandler: true,
    })
    assertScope(scope)
    return readResponse(response.data)
  } catch (error) {
    return responseFailure(error)
  }
}

export async function saveIPAllowlist(
  scope: IPAllowlistScope,
  allow_ips: string[]
): Promise<IPAllowlist> {
  assertScope(scope)
  try {
    const response = await withAuthenticatedCookieWrite(async (config) => {
      assertScope(scope)
      return api.put(
        '/api/user/self/ip-allowlist',
        { allow_ips },
        {
          ...config,
          authScope: scope,
          skipBusinessError: true,
          skipErrorHandler: true,
        }
      )
    }, false)
    assertScope(scope)
    return readResponse(response.data)
  } catch (error) {
    return responseFailure(error)
  }
}

export function parseIPAllowlistDraft(value: string): string[] {
  return [...new Set(value.split(/[,\s]+/u).filter(Boolean))]
}
