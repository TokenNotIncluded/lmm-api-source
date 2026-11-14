/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import axios from 'axios'
import { z } from 'zod'

import { api } from '@/lib/api'
import { AUTH_COOKIE_TIMEOUT_MS, withAuthCookieLock } from '@/lib/auth-cookie'
import {
  applyAuthBundle,
  applyPending2FAFlow,
  assertLoginResultCurrent,
  captureLoginIntentGuard,
  prepareLoginIntentVersion,
  isAuthBundle,
  type PreparedLoginIntent,
} from '@/lib/auth-session'

import {
  consumeFederatedCeremony,
  saveFederatedCeremony,
  validateFederatedStart,
  type FederatedProvider,
} from './lib/federated-oauth'
import { getAffiliateCode } from './lib/storage'
import type { ApiResponse } from './types'

const options = {
  authMode: 'anonymous' as const,
  skipAuthRefresh: true,
  skipErrorHandler: true,
  skipBusinessError: true,
  timeout: AUTH_COOKIE_TIMEOUT_MS,
}
const challengeSchema = z.discriminatedUnion('require_2fa', [
  z
    .object({
      require_2fa: z.literal(true),
      flow_token: z.string().regex(/^lmmf_[a-f0-9]{64}$/),
      expires_at: z.number().int().positive(),
    })
    .strict(),
  z
    .object({
      require_2fa: z.literal(false).optional(),
      require_passkey: z.literal(true),
      flow_token: z.string().regex(/^lmmp_[a-f0-9]{64}$/),
      expires_at: z.number().int().positive(),
      options: z.unknown(),
    })
    .strict(),
])
export function parseFederatedChallenge(value: unknown) {
  const parsed = challengeSchema.safeParse(value)
  if (!parsed.success) return null
  if (parsed.data.expires_at * 1000 <= Date.now()) {
    throw new Error('Login flow expired. Please sign in again.')
  }
  return parsed.data
}
export async function beginFederatedLogin(
  provider: FederatedProvider,
  acceptedLegal: boolean,
  signal?: AbortSignal,
  prepared?: PreparedLoginIntent
) {
  const ready = prepared ?? (await prepareLoginIntentVersion(signal))
  const assertCurrent = ready.assertCurrent
  assertCurrent()
  return withAuthCookieLock(async (transaction) => {
    assertCurrent()
    const res = await transaction.request(() =>
      api.post(
        `/api/oauth/federated/${provider.id}/begin`,
        { accepted_legal: acceptedLegal, affiliate: getAffiliateCode() || '' },
        options
      )
    )
    assertCurrent()
    if (signal?.aborted) throw new axios.CanceledError('Login cancelled')
    if (res.data?.success !== true) throw res.data
    const flow = validateFederatedStart(
      provider,
      res.data.data,
      window.location.origin
    )
    saveFederatedCeremony(
      window.sessionStorage,
      provider,
      flow,
      'login',
      ready.version
    )
    return flow
  }, signal)
}
export async function finishFederatedLogin(
  slug: string,
  input: { state: string; code: string; iss?: string },
  signal?: AbortSignal
): Promise<ApiResponse> {
  if (!input.code || input.code.length > 4096) throw new Error('Missing code')
  // Consume before dispatch: a failed/uncertain finish requires a fresh ceremony.
  const ceremony = consumeFederatedCeremony(
    window.sessionStorage,
    input.state,
    slug,
    'login'
  )
  const assertCurrent = captureLoginIntentGuard(ceremony.loginIntentVersion)
  assertCurrent()
  return withAuthCookieLock(async (transaction) => {
    assertCurrent()
    const response = await transaction.request(() =>
      api.post<ApiResponse>(
        `/api/oauth/federated/${ceremony.providerId}/finish`,
        input,
        options
      )
    )
    const cancellation = assertLoginResultCurrent(
      transaction,
      assertCurrent,
      response.data
    )
    if (cancellation) {
      await cancellation
    }
    const result = response.data
    if (result.success !== true && result.success !== false) {
      throw new Error('Login failed')
    }
    if (result.success === true) {
      const challenge = parseFederatedChallenge(result.data)
      if (challenge?.require_2fa) {
        applyPending2FAFlow(challenge.flow_token, challenge.expires_at)
      } else if (!challenge) {
        if (!isAuthBundle(result.data)) throw new Error('Login failed')
        applyAuthBundle(result.data)
      }
    }
    if (signal?.aborted) {
      throw new axios.CanceledError('Login cancelled after response')
    }
    return result
  }, signal)
}
