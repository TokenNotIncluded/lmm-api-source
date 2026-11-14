/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import axios from 'axios'
import { z } from 'zod'

import { useCredentialStore, type AuthBundle } from '@/stores/auth-store'

import {
  AuthCookieBlockedError,
  AUTH_COOKIE_TIMEOUT_MS,
  beginAuthCookieReload,
  getAuthCookieUncertaintyMarker,
  waitForAuthCookieIdle,
  withAuthCookieReconciliationLock,
} from './auth-cookie'
import { readLoginIntent } from './auth-login-intent'
import {
  applyAuthBundle,
  capturePasswordLoginGuard,
  invalidatePendingLogins,
  isAuthBundle,
} from './auth-session'

const HANDOFF_KEY = 'lmm:auth-cookie-reconciliation:v1'
let documentId: string | undefined
function recoveryDocumentId(): string {
  return (documentId ??= crypto.randomUUID())
}
const handoffSchema = z
  .object({
    documentId: z.uuid(),
    marker: z.string().startsWith('uncertain:').max(256),
    intent: z.uuid(),
    expectedSid: z
      .string()
      .regex(/^[a-f0-9]{64}$/)
      .optional(),
  })
  .strict()
const reconciliationSchema = z
  .object({
    success: z.literal(true),
    data: z.discriminatedUnion('status', [
      z
        .object({
          protocol: z.literal('core.browser-session-reconciliation.v1'),
          status: z.literal('current'),
          bundle: z.unknown(),
        })
        .strict(),
      z
        .object({
          protocol: z.literal('core.browser-session-reconciliation.v1'),
          status: z.literal('current_access_expired'),
        })
        .strict(),
    ]),
  })
  .strict()

export class AuthCookieSessionNeedsRenewalError extends Error {}

const renewalClient = axios.create({
  adapter: 'xhr',
  withCredentials: true,
  timeout: AUTH_COOKIE_TIMEOUT_MS,
})

/** The handoff contains no token and never removes the uncertainty marker. */
export async function prepareAuthCookieReconciliation(
  signal?: AbortSignal
): Promise<void> {
  const initial = useCredentialStore.getState().auth
  const scope = {
    userId: initial.user?.id,
    sid: initial.session?.sid,
    epoch: initial.accountWriteEpoch,
  }
  await waitForAuthCookieIdle()
  const marker = getAuthCookieUncertaintyMarker()
  if (!marker) throw new AuthCookieBlockedError('uncertain')
  await withAuthCookieReconciliationLock(
    marker,
    async () => {
      const current = useCredentialStore.getState().auth
      if (
        current.user?.id !== scope.userId ||
        current.session?.sid !== scope.sid ||
        current.accountWriteEpoch !== scope.epoch
      ) {
        throw new AuthCookieBlockedError('uncertain')
      }
      invalidatePendingLogins()
      const handoff = handoffSchema.parse({
        documentId: recoveryDocumentId(),
        marker,
        intent: readLoginIntent(window.localStorage),
        expectedSid: scope.sid,
      })
      const encoded = JSON.stringify(handoff)
      window.sessionStorage.setItem(HANDOFF_KEY, encoded)
      if (window.sessionStorage.getItem(HANDOFF_KEY) !== encoded) {
        throw new AuthCookieBlockedError('storage_unavailable')
      }
    },
    signal
  )
  signal?.throwIfAborted()
  if (!beginAuthCookieReload()) throw new AuthCookieBlockedError('uncertain')
  window.location.replace('/sign-in?reauth=1')
}

export function hasAuthCookieReconciliationHandoff(): boolean {
  try {
    const handoff = handoffSchema.safeParse(
      JSON.parse(window.sessionStorage.getItem(HANDOFF_KEY) ?? 'null')
    )
    return (
      handoff.success &&
      handoff.data.documentId !== recoveryDocumentId() &&
      handoff.data.intent === readLoginIntent(window.localStorage)
    )
  } catch {
    return false
  }
}

/** Only a new document may consume a user-requested recovery handoff. */
export function reconcileAuthCookieSession(signal?: AbortSignal) {
  return reconcileSession(false, signal)
}

/** Explicit user action; never used by bootstrap or the ordinary refresh runner. */
export function renewAuthCookieSession(signal?: AbortSignal) {
  return reconcileSession(true, signal)
}

async function reconcileSession(renew: boolean, signal?: AbortSignal) {
  const handoff = handoffSchema.parse(
    JSON.parse(window.sessionStorage.getItem(HANDOFF_KEY) ?? 'null')
  )
  if (
    handoff.documentId === recoveryDocumentId() ||
    readLoginIntent(window.localStorage) !== handoff.intent
  ) {
    throw new AuthCookieBlockedError('uncertain')
  }
  await waitForAuthCookieIdle()
  return withAuthCookieReconciliationLock(
    handoff.marker,
    async (confirm) => {
      signal?.throwIfAborted()
      const assertCurrent = capturePasswordLoginGuard(handoff.intent)
      const headers = handoff.expectedSid
        ? { 'X-Auth-Session': handoff.expectedSid }
        : undefined
      const readProof = async () => {
        signal?.throwIfAborted()
        const response = await fetch('/api/user/auth/reconcile', {
          method: 'POST',
          credentials: 'same-origin',
          cache: 'no-store',
          redirect: 'error',
          headers,
          signal: signal
            ? AbortSignal.any([signal, AbortSignal.timeout(10_000)])
            : AbortSignal.timeout(10_000),
        })
        if (response.status !== 200) {
          throw new AuthCookieBlockedError('uncertain')
        }
        const result = reconciliationSchema.parse(await response.json())
        signal?.throwIfAborted()
        assertCurrent()
        return result.data
      }
      const validateBundle = (value: unknown): AuthBundle => {
        if (
          !isAuthBundle(value) ||
          (handoff.expectedSid && value.session.sid !== handoff.expectedSid) ||
          value.token_type !== 'Bearer' ||
          !/^[a-f0-9]{64}$/.test(value.session.sid) ||
          value.access_expires_at <= Date.now() / 1000 ||
          value.session.expires_at <= Date.now() / 1000 ||
          !value.session.current
        ) {
          throw new AuthCookieBlockedError('uncertain')
        }
        return value
      }
      let proof = await readProof()
      let renewedBundle: AuthBundle | undefined
      if (proof.status === 'current_access_expired') {
        if (!renew) throw new AuthCookieSessionNeedsRenewalError()
        signal?.throwIfAborted()
        assertCurrent()
        // Preserve the original uncertain marker. Cancellation after dispatch
        // cannot release this lock before the native Cookie transport finishes.
        // The dedicated client has no automatic refresh/error/retry interceptors.
        const response = await renewalClient.post(
          '/api/user/auth/refresh',
          undefined,
          { headers }
        )
        signal?.throwIfAborted()
        assertCurrent()
        if (response.status !== 200 || response.data?.success !== true) {
          throw new AuthCookieBlockedError('uncertain')
        }
        renewedBundle = validateBundle(response.data.data)
        proof = await readProof()
      }
      if (proof.status !== 'current') {
        throw new AuthCookieBlockedError('uncertain')
      }
      const bundle = validateBundle(proof.bundle)
      signal?.throwIfAborted()
      assertCurrent()
      if (
        readLoginIntent(window.localStorage) !== handoff.intent ||
        (renewedBundle &&
          (bundle.session.sid !== renewedBundle.session.sid ||
            bundle.user.id !== renewedBundle.user.id ||
            bundle.access_token !== renewedBundle.access_token))
      ) {
        throw new AuthCookieBlockedError('uncertain')
      }
      confirm(() => applyAuthBundle(bundle))
      window.sessionStorage.removeItem(HANDOFF_KEY)
      return bundle
    },
    signal
  )
}
