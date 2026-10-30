/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.

For commercial licensing, please contact support@quantumnous.com
*/
import { z } from 'zod'

import { api, type ApiRequestConfig } from '@/lib/api'
import { AUTH_COOKIE_TIMEOUT_MS, withAuthCookieLock } from '@/lib/auth-cookie'
import {
  applyAuthBundle,
  applyPending2FAFlow,
  clearAuthentication,
  isAuthBundle,
  withAuthenticatedCookieWrite,
} from '@/lib/auth-session'
import { useAuthStore } from '@/stores/auth-store'

import type {
  SecurityProof,
  SecurityProofScope,
} from '../secure-verification/types'
import type { ApiResponse, PasskeyOptionsPayload, PasskeyStatus } from './types'

const credentialId = z
  .string()
  .regex(/^[1-9][0-9]{0,18}$/)
  .pipe(z.string().refine((value) => BigInt(value) <= 9223372036854775807n))
const statusSchema = z
  .object({
    enabled: z.boolean(),
    management: z.literal('core_recent_session'),
    last_used_at: z.string().nullable().optional(),
    credentials: z
      .array(
        z
          .object({
            id: credentialId,
            name: z.string(),
            created_at: z.string(),
            last_used_at: z.string().nullable(),
          })
          .strict()
      )
      .max(10),
  })
  .strict()
const challengeSchema = z
  .object({
    flow_token: z.string().regex(/^lmmp_[a-f0-9]{64}$/),
    options: z.unknown(),
    expires_at: z.number().int().positive(),
  })
  .strict()
const mfaSchema = z
  .object({
    require_2fa: z.literal(true),
    flow_token: z.string().regex(/^lmmf_[a-f0-9]{64}$/),
    expires_at: z.number().int().positive(),
  })
  .strict()

export type PasskeySessionScope = { userId: number; sessionId: string }

export function capturePasskeySession(): PasskeySessionScope {
  const auth = useAuthStore.getState().auth
  if (!auth.user || !auth.session) throw new Error('Session expired!')
  return { userId: auth.user.id, sessionId: auth.session.sid }
}

export function assertPasskeySession(scope: PasskeySessionScope): void {
  const auth = useAuthStore.getState().auth
  if (auth.user?.id !== scope.userId || auth.session?.sid !== scope.sessionId) {
    throw new Error('Session expired!')
  }
}

function commitManagementResponse(
  response: ApiResponse,
  scope: PasskeySessionScope
) {
  assertPasskeySession(scope)
  if (!response.success) return response
  if (!isAuthBundle(response.data) || response.data.user.id !== scope.userId) {
    clearAuthentication(true)
    throw new Error('Invalid authentication rotation response')
  }
  applyAuthBundle(response.data)
  return response
}

function proofHeaders(proofToken?: string): Record<string, string> | undefined {
  return proofToken ? { 'X-Security-Proof': proofToken } : undefined
}

export async function getPasskeyStatus(
  config: Pick<ApiRequestConfig, 'skipErrorHandler'> = {}
): Promise<ApiResponse<PasskeyStatus>> {
  const scope = capturePasskeySession()
  const res = await api.get<ApiResponse<PasskeyStatus>>('/api/user/passkey', {
    ...config,
    authScope: scope,
    timeout: AUTH_COOKIE_TIMEOUT_MS,
  })
  assertPasskeySession(scope)
  if (res.data.success) res.data.data = statusSchema.parse(res.data.data)
  return res.data
}

export async function beginPasskeyRegistration(
  scope: PasskeySessionScope,
  proofToken?: string
): Promise<ApiResponse<PasskeyOptionsPayload>> {
  assertPasskeySession(scope)
  return withAuthenticatedCookieWrite(async (config) => {
    assertPasskeySession(scope)
    const res = await api.post<ApiResponse<PasskeyOptionsPayload>>(
      '/api/user/passkey/register/begin',
      undefined,
      { ...config, headers: proofHeaders(proofToken), skipErrorHandler: true }
    )
    assertPasskeySession(scope)
    if (res.data.success) res.data.data = challengeSchema.parse(res.data.data)
    return res.data
  })
}

export async function finishPasskeyRegistration(
  flowToken: string,
  payload: Record<string, unknown>,
  name: string,
  scope: PasskeySessionScope,
  proofToken?: string
): Promise<ApiResponse> {
  assertPasskeySession(scope)
  return withAuthenticatedCookieWrite(async (config) => {
    assertPasskeySession(scope)
    const res = await api.post<ApiResponse>(
      '/api/user/passkey/register/finish',
      { flow_token: flowToken, credential: payload, name },
      { ...config, headers: proofHeaders(proofToken), skipErrorHandler: true }
    )
    return commitManagementResponse(res.data, scope)
  })
}

export async function deletePasskey(
  id: string,
  proofToken?: string
): Promise<ApiResponse> {
  const scope = capturePasskeySession()
  credentialId.parse(id)
  return withAuthenticatedCookieWrite(async (config) => {
    assertPasskeySession(scope)
    const res = await api.delete<ApiResponse>(`/api/user/passkey/${id}`, {
      ...config,
      headers: proofHeaders(proofToken),
      skipErrorHandler: true,
    })
    return commitManagementResponse(res.data, scope)
  })
}

export async function beginPasskeyLogin(
  username: string,
  assertCurrent: () => void
): Promise<ApiResponse<PasskeyOptionsPayload>> {
  return withAuthCookieLock(async (transaction) => {
    assertCurrent()
    const res = await transaction.request(() =>
      api.post<ApiResponse<PasskeyOptionsPayload>>(
        '/api/user/passkey/login/begin',
        { username: username.trim() || undefined },
        {
          skipAuthRefresh: true,
          skipErrorHandler: true,
          timeout: AUTH_COOKIE_TIMEOUT_MS,
        }
      )
    )
    assertCurrent()
    if (res.data.success) res.data.data = challengeSchema.parse(res.data.data)
    return res.data
  })
}

export async function finishPasskeyLogin(
  flowToken: string,
  payload: Record<string, unknown>,
  assertCurrent: () => void
): Promise<ApiResponse> {
  return withAuthCookieLock(async (transaction) => {
    assertCurrent()
    const res = await transaction.request(() =>
      api.post<ApiResponse>(
        '/api/user/passkey/login/finish',
        { flow_token: flowToken, credential: payload },
        {
          skipAuthRefresh: true,
          skipErrorHandler: true,
          timeout: AUTH_COOKIE_TIMEOUT_MS,
        }
      )
    )
    assertCurrent()
    if (res.data.success) {
      const mfa = mfaSchema.safeParse(res.data.data)
      if (mfa.success) {
        applyPending2FAFlow(mfa.data.flow_token, mfa.data.expires_at)
      } else {
        if (!isAuthBundle(res.data.data)) throw new Error('Login failed')
        applyAuthBundle(res.data.data)
      }
    }
    return res.data
  })
}

export async function beginPasskeyVerification(
  scope: SecurityProofScope
): Promise<ApiResponse<PasskeyOptionsPayload>> {
  const res = await api.post<ApiResponse<PasskeyOptionsPayload>>(
    '/api/user/passkey/verify/begin',
    { scope }
  )
  return res.data
}

export async function finishPasskeyVerification(
  flowToken: string,
  payload: Record<string, unknown>
): Promise<ApiResponse<SecurityProof>> {
  const res = await api.post<ApiResponse<SecurityProof>>(
    '/api/user/passkey/verify/finish',
    { flow_token: flowToken, credential: payload }
  )
  return res.data
}
