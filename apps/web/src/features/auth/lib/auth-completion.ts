/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import axios from 'axios'
import type { TFunction } from 'i18next'
import { toast } from 'sonner'

import {
  AuthCookieBlockedError,
  getAuthCookieBlockedState,
} from '@/lib/auth-cookie'
import { applyPending2FAFlow, isAuthBundle } from '@/lib/auth-session'
import {
  prepareCredentialRequestOptions,
  requestPasskeyAuthentication,
} from '@/lib/passkey'
import { rateLimitMessage } from '@/lib/request-rate-limit'
import {
  getServerErrorMessageKey,
  getSafeOAuthAccessDeniedMessage,
} from '@/lib/server-error-message'
import { useAuthStore, type AuthBundle } from '@/stores/auth-store'

import { finishPasskeyLogin } from '../passkey'
import {
  hasVerifiedAccountAppeal,
  setVerifiedAccountAppealHint,
} from './account-appeal'

class AuthCompletionError extends Error {}

export type LoginPasskeyChallenge = {
  options: unknown
  flowToken: string
  expiresAt?: number
}
type CompletionResponse = { success: boolean; data?: unknown }
export function hasCurrentLoginBundle(bundle: AuthBundle) {
  const auth = useAuthStore.getState().auth
  return (
    auth.user?.id === bundle.user.id &&
    auth.session?.sid === bundle.session.sid &&
    auth.accessToken === bundle.access_token
  )
}
// Presentation callbacks reuse useAuthRedirect; all authority remains in auth-session.
export async function completeAuthResponse(
  response: CompletionResponse,
  handlers: {
    alreadyApplied: boolean
    signal?: AbortSignal
    onBundle: (
      bundle: AuthBundle,
      alreadyApplied: boolean
    ) => void | Promise<void>
    onMFA: () => void
    onPasskey?: (challenge: LoginPasskeyChallenge) => void | Promise<void>
  }
): Promise<boolean> {
  if (handlers.signal?.aborted || getAuthCookieBlockedState()) return true
  if (!response.success) return false
  const data = response.data as Record<string, unknown> | undefined
  if (data?.require_2fa === true || data?.require_passkey === true) {
    if (typeof data.flow_token !== 'string' || !data.flow_token) {
      throw new AuthCompletionError('Login flow expired. Please sign in again.')
    }
    const expiresAt =
      typeof data.expires_at === 'number' ? data.expires_at : undefined
    if (expiresAt !== undefined && expiresAt * 1000 <= Date.now()) {
      throw new AuthCompletionError('Login flow expired. Please sign in again.')
    }
    if (data.require_2fa === true) {
      if (!handlers.alreadyApplied) {
        applyPending2FAFlow(data.flow_token, expiresAt)
      }
      if (
        useAuthStore.getState().auth.pending2FAFlowToken === data.flow_token
      ) {
        handlers.onMFA()
      }
      return true
    }
    prepareCredentialRequestOptions(data.options)
    if (!handlers.onPasskey) return false
    await handlers.onPasskey({
      options: data.options,
      flowToken: data.flow_token,
      expiresAt,
    })
    return true
  }
  if (!isAuthBundle(response.data)) return false
  if (handlers.alreadyApplied && !hasCurrentLoginBundle(response.data)) {
    return true
  }
  await handlers.onBundle(response.data, handlers.alreadyApplied)
  return true
}
export async function verifyAuthPasskey(
  challenge: LoginPasskeyChallenge,
  assertCurrent: () => void
) {
  if (
    challenge.expiresAt !== undefined &&
    challenge.expiresAt * 1000 <= Date.now()
  ) {
    throw new AuthCompletionError('Login flow expired. Please sign in again.')
  }
  const credential = await requestPasskeyAuthentication(challenge.options)
  assertCurrent()
  if (!credential) return null
  return finishPasskeyLogin(challenge.flowToken, credential, assertCurrent)
}
export function showVerifiedAuthAppeal(
  value: unknown,
  t: TFunction,
  onAppeal?: (value: unknown) => boolean | void
): boolean {
  if (!hasVerifiedAccountAppeal(value)) return false
  setVerifiedAccountAppealHint(true)
  if (onAppeal?.(value) !== true) {
    toast.error(
      t(
        'Submit an appeal using your verified identity. No local password is needed.'
      )
    )
  }
  return true
}
// A published Cookie block is presented once by AuthCookieRecovery.
export function shouldSuppressAuthError(value?: unknown): boolean {
  return axios.isCancel(value) || Boolean(getAuthCookieBlockedState())
}

// For callers with their own safe ordinary-error presenter. Only the current
// notification owner calls this; notify=false flows leave unpublished errors
// to their caller, preserving the localized fallback when no banner exists.
export function reportOwnedAuthError(value: unknown, t: TFunction): boolean {
  if (shouldSuppressAuthError(value)) return true
  if (!(value instanceof AuthCookieBlockedError)) return false
  reportAuthError(value, t, t(value.message))
  return true
}

export function reportPasskeyCancellation(
  value: unknown,
  t: TFunction
): boolean {
  if (shouldSuppressAuthError(value)) return true
  if (
    value !== null &&
    !(value instanceof Error && value.name === 'NotAllowedError')
  ) {
    return false
  }
  toast.info(t('Passkey login was cancelled'))
  return true
}

export function reportAuthError(
  value: unknown,
  t: TFunction,
  fallback: string,
  onAppeal?: (value: unknown) => boolean | void,
  mode: 'default' | 'password' = 'default'
) {
  if (shouldSuppressAuthError(value)) return
  if (showVerifiedAuthAppeal(value, t, onAppeal)) return
  if (value instanceof AuthCompletionError) {
    toast.error(t(value.message))
    return
  }
  if (value instanceof AuthCookieBlockedError) {
    // The root recovery alert owns an active Cookie block and its action.
    // A duplicate toast can cover that persistent guidance on narrow screens.
    if (!getAuthCookieBlockedState()) toast.error(t(value.message))
    return
  }
  const messageKey = getServerErrorMessageKey(value)
  const safeKey =
    messageKey === 'Session expired!'
      ? null
      : messageKey ===
          'This account is disabled. Use the appeal form below to ask an administrator to restore access.'
        ? 'This user account is disabled.'
        : messageKey
  const denialMessage = getSafeOAuthAccessDeniedMessage(value)
  const limited = rateLimitMessage(value)
  const status = axios.isAxiosError(value) ? value.response?.status : undefined
  const message =
    mode === 'password' && status === 401
      ? fallback
      : safeKey
        ? t(safeKey)
        : mode === 'password' && status === 403
          ? t('Request failed')
          : mode === 'password' && status === 503
            ? t(
                'The service is temporarily unavailable. Please try again later.'
              )
            : fallback
  toast.error(
    limited ?? denialMessage ?? message,
    limited ? { id: 'request-rate-limited' } : undefined
  )
}
