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
import type { QueryClient } from '@tanstack/react-query'
import axios, { type AxiosAdapter } from 'axios'
import { t } from 'i18next'

import {
  AUTH_COOKIE_TIMEOUT_MS,
  AuthCookieBlockedError,
  getAuthCookieBlockedState,
  withAuthCookieActivity,
  withAuthCookieLock,
  type AuthCookieTransaction,
} from '@/lib/auth-cookie'
import {
  ensureLoginInitialization,
  formatLoginIntentVersion,
  isLoginIntentVersionCurrent,
  parseLoginIntentVersion,
  readLoginInitialization,
  type IntentStorage,
  invalidateLoginIntent,
  readLoginIntent,
} from '@/lib/auth-login-intent'
import { publishAuthSessionEvent } from '@/lib/auth-session-sync'
import {
  isRateLimitedError,
  rateLimitWaitSeconds,
} from '@/lib/request-rate-limit'
import {
  useAuthStore,
  type AuthBootstrapState,
  type AuthBundle,
  type AuthUser,
  type LoginSession,
} from '@/stores/auth-store'

export type RefreshOutcome =
  | { kind: 'authenticated'; bundle: AuthBundle }
  | { kind: 'anonymous' }
  | { kind: 'transient_error'; error: unknown }
  | { kind: 'out_of_sync'; code?: string }

interface AuthRefreshHTTPResponse {
  status: number
  data?: unknown
  error?: unknown
}

export interface AuthRefreshRuntime {
  request: (expectedSID?: string) => Promise<AuthRefreshHTTPResponse>
  getExpectedSID: () => string | undefined
  parseBundle: (value: unknown) => AuthBundle | null
  acceptBundle: (bundle: AuthBundle) => void
  clear: (synchronizeTabs: boolean, bootstrapState?: AuthBootstrapState) => void
  markTransient: () => void
  wait: (delay: number) => Promise<void>
  isCurrent?: () => boolean
}

interface AuthTokenRotation {
  access_token: string
  token_type: string
  access_expires_at: number
  session: LoginSession
}

class AuthRotationError extends Error {
  constructor(message: string) {
    super(message)
    this.name = 'AuthRotationError'
  }
}

const authClient = axios.create({
  baseURL: '',
  withCredentials: true,
  headers: {
    'Cache-Control': 'no-store',
  },
})

/** Installs the isolated refresh transport used by the development persona lab. */
export function setDevelopmentAuthRefreshAdapter(adapter: AxiosAdapter): void {
  authClient.defaults.adapter = adapter
}

const refreshRaceDelays = [80, 200, 500] as const
let refreshPromise: Promise<RefreshOutcome> | null = null
type AuthenticationGeneration = {
  authEpoch: number
  accountWriteEpoch: number
}
type RefreshAccountWrite = {
  before?: AuthenticationGeneration
  after?: AuthenticationGeneration
}
let refreshAccountWrite: RefreshAccountWrite | null = null
let refreshCooldown: {
  epoch: number
  sid: string | undefined
  until: number
  outcome: RefreshOutcome
} | null = null
let authEpoch = 0
let authCache: QueryClient | null = null

function authenticationGeneration(): AuthenticationGeneration {
  return {
    authEpoch,
    accountWriteEpoch: useAuthStore.getState().auth.accountWriteEpoch,
  }
}

function sameGeneration(
  left: AuthenticationGeneration,
  right?: AuthenticationGeneration
): boolean {
  return (
    left.authEpoch === right?.authEpoch &&
    left.accountWriteEpoch === right?.accountWriteEpoch
  )
}

function currentLoginIntentVersion(): string {
  try {
    return readLoginIntent(
      typeof window === 'undefined' ? undefined : window.localStorage
    )
  } catch {
    throw new AuthCookieBlockedError(
      getAuthCookieBlockedState() ?? 'storage_unavailable'
    )
  }
}

export interface PreparedLoginIntent {
  readonly version: string
  readonly assertCurrent: () => void
}

function loginIntentStorage<T>(operation: (storage?: IntentStorage) => T): T {
  try {
    return operation(
      typeof window === 'undefined' ? undefined : window.localStorage
    )
  } catch {
    throw new AuthCookieBlockedError(
      getAuthCookieBlockedState() ?? 'storage_unavailable'
    )
  }
}

export async function prepareLoginIntentVersion(
  signal?: AbortSignal
): Promise<PreparedLoginIntent> {
  // Capture BOTH cancellation and identity before the first init/lock await.
  const cancellation = currentLoginIntentVersion()
  const assertClick = capturePasswordLoginGuard(cancellation)
  const ready = (initialization: string): PreparedLoginIntent => {
    const version = loginIntentStorage(() =>
      formatLoginIntentVersion(initialization, cancellation)
    )
    const assertCurrent = () => {
      assertClick()
      if (
        !loginIntentStorage((storage) =>
          isLoginIntentVersionCurrent(version, storage)
        )
      ) {
        throw new axios.CanceledError('Login superseded')
      }
    }
    assertCurrent()
    return { version, assertCurrent }
  }
  if (signal?.aborted) throw new axios.CanceledError('Login cancelled')
  assertClick()
  const existing = loginIntentStorage(readLoginInitialization)
  if (existing !== null) return ready(existing)
  return withAuthCookieLock(async () => {
    assertClick()
    // Recheck under the same Cookie lock; no HTTP or nested acquisition here.
    const initialization = loginIntentStorage(ensureLoginInitialization)
    assertClick()
    return ready(initialization)
  }, signal)
}

/** Redirects require the original I+C, never a missing-field/current-value fallback. */
export function captureLoginIntentGuard(
  version: string | undefined
): () => void {
  const receipt = parseLoginIntentVersion(version)
  if (!receipt) throw new axios.CanceledError('Login superseded')
  const assertIdentity = capturePasswordLoginGuard(receipt.cancellation)
  return () => {
    assertIdentity()
    if (
      !loginIntentStorage((storage) =>
        isLoginIntentVersionCurrent(version, storage)
      )
    ) {
      throw new axios.CanceledError('Login superseded')
    }
  }
}

/** Cancel pending logins before waiting behind any queued Cookie writer. */
export function invalidatePendingLogins(): void {
  try {
    invalidateLoginIntent(
      typeof window === 'undefined' ? undefined : window.localStorage
    )
  } catch {
    throw new AuthCookieBlockedError(
      getAuthCookieBlockedState() ?? 'storage_unavailable'
    )
  }
}

/** Only an already-running passive refresh may precede this explicit login. */
export function capturePasswordLoginGuard(
  loginIntentVersion = currentLoginIntentVersion()
): () => void {
  const initial = authenticationGeneration()
  const passive = refreshPromise ? refreshAccountWrite : null
  return () => {
    if (currentLoginIntentVersion() !== loginIntentVersion) {
      throw new axios.CanceledError('Login superseded')
    }
    const current = authenticationGeneration()
    if (
      sameGeneration(current, initial) ||
      (sameGeneration(initial, passive?.before) &&
        sameGeneration(current, passive?.after))
    ) {
      return
    }
    throw new axios.CanceledError('Login superseded')
  }
}

function browserAuthURL(path: string): string {
  // Browser auth is same-origin (the Rsbuild SERVER_URL configures its dev
  // proxy, not a browser API base). Preserve the client's configured path;
  // never silently send session credentials to a different configured origin.
  const url = new URL(authClient.getUri({ url: path }), window.location.href)
  if (url.origin !== window.location.origin) {
    throw new Error(t('Request failed'))
  }
  return url.href
}

/** Revoke exactly the credential's family, without sending or changing Cookies. */
async function revokeBrowserSession(
  transaction: AuthCookieTransaction,
  sid: string,
  accessToken: string
): Promise<void> {
  const response = await transaction.request(() =>
    authClient.delete(
      browserAuthURL(`/api/user/sessions/${encodeURIComponent(sid)}`),
      {
        // XHR withCredentials=false still sends same-origin Cookies. The Fetch
        // adapter maps false to credentials:'omit', including Set-Cookie handling.
        adapter: 'fetch',
        withCredentials: false,
        timeout: AUTH_COOKIE_TIMEOUT_MS,
        fetchOptions: { redirect: 'error', cache: 'no-store' },
        headers: {
          Authorization: `Bearer ${accessToken}`,
          'X-Auth-Session': sid,
        },
      }
    )
  )
  if (
    response.data?.success !== true ||
    response.data.data?.revoked_sid !== sid ||
    response.data.data?.current !== true
  ) {
    throw new Error(t('Request failed'))
  }
}

/** An expired A bearer is not proof that A's refresh family was revoked. */
export async function revokeLogoutSession(auth: {
  userId: number
  sid: string
  accessToken: string | null
  accessExpiresAt: number | null
}): Promise<void> {
  let accessToken = auth.accessToken
  if (
    !accessToken ||
    !auth.accessExpiresAt ||
    auth.accessExpiresAt <= Date.now() / 1000
  ) {
    // This can recover A only if its refresh Cookie still exists. A Cookie
    // already replaced by B cannot be recovered from an expired bearer.
    const response = await withAuthCookieLock((transaction) =>
      requestRefresh(auth.sid, transaction)
    )
    const data = isRecord(response.data) ? response.data : undefined
    if (
      data?.success !== true ||
      !isAuthBundle(data.data) ||
      data.data.session.sid !== auth.sid ||
      data.data.user.id !== auth.userId
    ) {
      throw response.error ?? new Error(t('Session expired!'))
    }
    accessToken = data.data.access_token
  }
  // A may already be a rotated historical bearer. Only the purpose-specific
  // logout path accepts that proof during its original lifetime. Omit Cookies
  // so a newer B/C cannot select a different family or be cleared by Set-Cookie.
  // This request cannot read or write Cookies. Cookie uncertainty or queued
  // writers must not prevent revoking the family selected at the logout click.
  const response = await authClient.post(
    browserAuthURL('/api/user/auth/logout'),
    undefined,
    {
      adapter: 'fetch',
      withCredentials: false,
      timeout: AUTH_COOKIE_TIMEOUT_MS,
      fetchOptions: { redirect: 'error', cache: 'no-store' },
      headers: {
        Authorization: `Bearer ${accessToken}`,
        'X-Auth-Session': auth.sid,
      },
    }
  )
  // Idempotent success without proof deliberately has no revocation receipt.
  if (
    response.data?.success !== true ||
    response.data.data?.revoked_sid !== auth.sid
  ) {
    throw new Error(t('Request failed'))
  }
}

/** A cancelled response may already have minted a real server-side family. */
// Keep successful validation synchronous with its caller’s bundle commit.
// Only rejected results require asynchronous server-side compensation.
export function assertLoginResultCurrent(
  transaction: AuthCookieTransaction,
  assertCurrent: () => void,
  result: unknown
): Promise<never> | undefined {
  try {
    assertCurrent()
    return undefined
  } catch (error) {
    return (async () => {
      if (
        isRecord(result) &&
        result.success === true &&
        isAuthBundle(result.data)
      ) {
        const bundle = result.data
        await revokeBrowserSession(
          transaction,
          bundle.session.sid,
          bundle.access_token
        )
        try {
          const response = await transaction.request(() =>
            authClient.post(
              browserAuthURL('/api/user/auth/logout'),
              undefined,
              {
                adapter: 'fetch',
                withCredentials: true,
                timeout: AUTH_COOKIE_TIMEOUT_MS,
                fetchOptions: { redirect: 'error', cache: 'no-store' },
                headers: {
                  Authorization: `Bearer ${bundle.access_token}`,
                  'X-Auth-Session': bundle.session.sid,
                },
              }
            )
          )
          if (response.data?.success !== true) {
            throw new Error(t('Request failed'))
          }
        } catch (cleanupError) {
          // B is confirmed revoked. A newer Cookie C must remain untouched.
          if (
            !(
              axios.isAxiosError(cleanupError) &&
              cleanupError.response?.status === 409 &&
              cleanupError.response.data?.code === 'AUTH_SESSION_MISMATCH'
            )
          ) {
            throw cleanupError
          }
        }
      }
      throw error
    })()
  }
}

class AuthRefreshSupersededError extends Error {
  constructor() {
    super('Authentication refresh was superseded')
    this.name = 'AuthRefreshSupersededError'
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === 'object'
}

function isAuthUser(value: unknown): value is AuthUser {
  if (!isRecord(value)) return false
  return (
    Number.isInteger(value.id) &&
    Number(value.id) > 0 &&
    typeof value.username === 'string' &&
    typeof value.role === 'number'
  )
}

function isLoginSession(value: unknown): value is LoginSession {
  if (!isRecord(value)) return false
  return (
    typeof value.sid === 'string' &&
    value.sid.length > 0 &&
    typeof value.current === 'boolean' &&
    typeof value.login_method === 'string' &&
    typeof value.ip === 'string' &&
    typeof value.user_agent === 'string' &&
    typeof value.created_at === 'number' &&
    typeof value.last_active_at === 'number' &&
    typeof value.expires_at === 'number'
  )
}

function hasValidTokenFields(value: Record<string, unknown>): boolean {
  return (
    typeof value.access_token === 'string' &&
    value.access_token.length > 0 &&
    typeof value.token_type === 'string' &&
    value.token_type.length > 0 &&
    typeof value.access_expires_at === 'number' &&
    Number.isFinite(value.access_expires_at) &&
    value.access_expires_at > 0
  )
}

export function isAuthBundle(value: unknown): value is AuthBundle {
  if (!isRecord(value)) return false
  return (
    hasValidTokenFields(value) &&
    isAuthUser(value.user) &&
    isLoginSession(value.session)
  )
}

function isAuthTokenRotation(value: unknown): value is AuthTokenRotation {
  return (
    isRecord(value) &&
    hasValidTokenFields(value) &&
    value.token_type === 'Bearer' &&
    isLoginSession(value.session) &&
    value.session.current
  )
}

// An anonymous capability read can already be mounted when bootstrap refresh
// returns 401. Removing its query cancels the request and strands its observer
// on a pending result. Only keep that active public read; all user-scoped
// status, permissions, financial data and mutations must leave with the session.
function clearSessionCache(queryClient: QueryClient): void {
  queryClient.removeQueries({
    predicate: (query) =>
      !(
        query.isActive() &&
        query.queryKey.length === 2 &&
        query.queryKey[0] === 'status' &&
        query.queryKey[1] === 'anonymous'
      ),
  })
  queryClient.getMutationCache().clear()
}

function assertAuthCommitAvailable(): void {
  const blocked = getAuthCookieBlockedState()
  if (blocked) throw new AuthCookieBlockedError(blocked)
}

export function applyAuthBundle(
  bundle: AuthBundle,
  synchronizeTabs = true
): void {
  assertAuthCommitAvailable()
  const previousSID = useAuthStore.getState().auth.session?.sid
  if (previousSID && previousSID !== bundle.session.sid) {
    if (authCache) clearSessionCache(authCache)
  }
  refreshCooldown = null
  authEpoch += 1
  useAuthStore.getState().auth.setBundle(bundle)
  if (synchronizeTabs && previousSID !== bundle.session.sid) {
    publishAuthSessionEvent('authenticated', bundle.session.sid)
  }
}

export function applyPending2FAFlow(flow: string, expiresAt?: number): void {
  assertAuthCommitAvailable()
  // A refresh queued before the password challenge cannot consume the ceremony.
  authEpoch += 1
  useAuthStore.getState().auth.setPending2FAFlowToken(flow, expiresAt)
}

export function applyAuthRotation(value: unknown): void {
  if (!isAuthTokenRotation(value)) {
    throw new AuthRotationError('Invalid authentication rotation response')
  }

  const auth = useAuthStore.getState().auth
  if (!auth.user || !auth.session) {
    throw new AuthRotationError('Authentication rotation has no active session')
  }
  if (value.session.sid !== auth.session.sid) {
    throw new AuthRotationError('Authentication rotation session mismatch')
  }

  applyAuthBundle(
    {
      access_token: value.access_token,
      token_type: value.token_type,
      access_expires_at: value.access_expires_at,
      session: value.session,
      user: auth.user,
    },
    false
  )
}

export function clearAuthentication(
  synchronizeTabs = true,
  bootstrapState: AuthBootstrapState = 'complete',
  preserveValidPending2FA = false
): void {
  const sid = useAuthStore.getState().auth.session?.sid
  if (authCache) clearSessionCache(authCache)
  refreshCooldown = null
  authEpoch += 1
  useAuthStore.getState().auth.reset(bootstrapState, preserveValidPending2FA)
  if (synchronizeTabs && sid) {
    publishAuthSessionEvent('signed_out', sid)
  }
}

export type LocalAuthRecoveryScope = Readonly<{
  userId: number | undefined
  sessionId: string | undefined
  authEpoch: number
  accountWriteEpoch: number
}>

export function captureLocalAuthRecoveryScope(): LocalAuthRecoveryScope {
  const auth = useAuthStore.getState().auth
  return {
    userId: auth.user?.id,
    sessionId: auth.session?.sid,
    ...authenticationGeneration(),
  }
}

/** Explicit local UI recovery. Never confirms server revocation or unlocks Cookies. */
export function endLocalAuthSessionForRecovery(scope: LocalAuthRecoveryScope): {
  kind: 'ended' | 'superseded' | 'not_blocked'
  serverLogoutConfirmed: false
} {
  const current = captureLocalAuthRecoveryScope()
  if (
    !sameGeneration(current, scope) ||
    current.userId !== scope.userId ||
    current.sessionId !== scope.sessionId
  ) {
    return { kind: 'superseded', serverLogoutConfirmed: false }
  }
  if (!getAuthCookieBlockedState()) {
    return { kind: 'not_blocked', serverLogoutConfirmed: false }
  }
  // This check and reset are synchronous. Other tabs keep their current SID;
  // late writers in this document remain fenced by the unchanged Cookie state.
  clearAuthentication(false, 'complete')
  return { kind: 'ended', serverLogoutConfirmed: false }
}

/** Binds the single application query cache to the authentication lifecycle. */
export function bindAuthCache(queryClient: QueryClient): () => void {
  authCache = queryClient
  return () => {
    if (authCache === queryClient) authCache = null
  }
}

export function clearAuthenticatedClientState(
  queryClient: QueryClient,
  synchronizeTabs = true
): void {
  if (queryClient !== authCache) clearSessionCache(queryClient)
  clearAuthentication(synchronizeTabs)
}

/** A deleted account cannot invalidate a later login, even if logout failed. */
export function clearAuthenticationIfCurrent(scope: {
  userId: number | undefined
  sessionId: string | undefined
}): void {
  const auth = useAuthStore.getState().auth
  if (auth.user?.id === scope.userId && auth.session?.sid === scope.sessionId) {
    clearAuthentication()
  }
}

function waitForRefreshRace(delay: number): Promise<void> {
  return new Promise((resolve) => globalThis.setTimeout(resolve, delay))
}

export function createRefreshRunner(
  runtime: AuthRefreshRuntime
): () => Promise<RefreshOutcome> {
  const superseded = (): RefreshOutcome => ({
    kind: 'transient_error',
    error: new AuthRefreshSupersededError(),
  })
  const run = async (raceAttempt: number): Promise<RefreshOutcome> => {
    if (runtime.isCurrent && !runtime.isCurrent()) return superseded()
    const expectedSID = runtime.getExpectedSID()
    const response = await runtime.request(expectedSID)
    if (runtime.isCurrent && !runtime.isCurrent()) return superseded()
    const responseData = isRecord(response.data) ? response.data : undefined
    const code =
      typeof responseData?.code === 'string' ? responseData.code : undefined
    const bundle = runtime.parseBundle(responseData?.data)
    if (responseData?.success === true && bundle) {
      if (expectedSID && bundle.session.sid !== expectedSID) {
        runtime.clear(false)
        return { kind: 'out_of_sync', code: 'AUTH_SESSION_MISMATCH' }
      }
      runtime.acceptBundle(bundle)
      return { kind: 'authenticated', bundle }
    }

    if (response.status === 409 && code === 'AUTH_REFRESH_RACE') {
      const delay = refreshRaceDelays[raceAttempt]
      if (delay !== undefined) {
        await runtime.wait(delay)
        return run(raceAttempt + 1)
      }
      runtime.clear(false)
      return { kind: 'out_of_sync', code }
    }

    if (response.status === 409 && code === 'AUTH_SESSION_MISMATCH') {
      // Removing expectedSID here would let a late foreign cookie family
      // silently become this tab's signed-in account.
      runtime.clear(false)
      return { kind: 'out_of_sync', code }
    }

    if (response.status === 401) {
      runtime.clear(true)
      return { kind: 'anonymous' }
    }

    if (
      !response.status ||
      response.status === 408 ||
      response.status === 425 ||
      response.status === 429 ||
      response.status >= 500
    ) {
      runtime.markTransient()
      return {
        kind: 'transient_error',
        error: response.error ?? response.data,
      }
    }

    runtime.clear(false)
    return {
      kind: 'out_of_sync',
      code: code ?? 'AUTH_INVALID_REFRESH_RESPONSE',
    }
  }

  return () => run(0)
}

async function requestRefresh(
  expectedSID: string | undefined,
  transaction: AuthCookieTransaction
): Promise<AuthRefreshHTTPResponse> {
  try {
    const response = await transaction.request(() =>
      authClient.post('/api/user/auth/refresh', undefined, {
        headers: expectedSID ? { 'X-Auth-Session': expectedSID } : undefined,
        timeout: AUTH_COOKIE_TIMEOUT_MS,
      })
    )
    return { status: response.status, data: response.data }
  } catch (error: unknown) {
    if (!axios.isAxiosError(error)) return { status: 0, error }
    return {
      status: error.response?.status ?? 0,
      data: error.response?.data,
      error,
    }
  }
}

function runRefresh(
  refreshEpoch: number,
  transaction: AuthCookieTransaction,
  accountWrite?: RefreshAccountWrite
): Promise<RefreshOutcome> {
  const commitBundle = (bundle: AuthBundle) => {
    const before = authenticationGeneration()
    applyAuthBundle(bundle, false)
    const after = authenticationGeneration()
    if (
      accountWrite &&
      after.authEpoch === before.authEpoch + 1 &&
      after.accountWriteEpoch === before.accountWriteEpoch + 1
    ) {
      accountWrite.before = before
      // A synchronous store listener's additional write must not be
      // attributed to this refresh. Both generations advance exactly once.
      accountWrite.after = after
    }
  }
  return createRefreshRunner({
    request: (sid) => requestRefresh(sid, transaction),
    getExpectedSID: () => useAuthStore.getState().auth.session?.sid,
    parseBundle: (value) => (isAuthBundle(value) ? value : null),
    acceptBundle: commitBundle,
    clear: (synchronizeTabs, bootstrapState) => {
      if (!synchronizeTabs && bootstrapState === 'idle') {
        useAuthStore.getState().auth.reset('idle')
        return
      }
      const auth = useAuthStore.getState().auth
      if (
        synchronizeTabs &&
        !auth.user &&
        !auth.session &&
        !auth.accessToken &&
        auth.accessExpiresAt === null &&
        !auth.pending2FAFlowToken &&
        auth.pending2FAFlowExpiresAt === null
      ) {
        // An anonymous refresh rejection confirms the existing identity. It must
        // not invalidate an explicit login queued behind this Cookie operation.
        // Explicit logout and rejection of an existing identity still reset it.
        if (authCache) clearSessionCache(authCache)
        refreshCooldown = null
        auth.setBootstrapState(bootstrapState ?? 'complete')
        return
      }
      clearAuthentication(synchronizeTabs, bootstrapState)
    },
    markTransient: () => useAuthStore.getState().auth.setBootstrapState('idle'),
    wait: waitForRefreshRace,
    isCurrent: () => authEpoch === refreshEpoch,
  })()
}

async function performRefreshWithBrowserLock(
  refreshEpoch: number,
  accountWrite: RefreshAccountWrite
): Promise<RefreshOutcome> {
  try {
    return await withAuthCookieLock((transaction) =>
      runRefresh(refreshEpoch, transaction, accountWrite)
    )
  } catch (error: unknown) {
    if (authEpoch === refreshEpoch) {
      useAuthStore.getState().auth.setBootstrapState('idle')
    }
    return { kind: 'transient_error', error }
  }
}

function currentRefreshCooldown(epoch: number, sid: string | undefined) {
  if (
    refreshCooldown?.epoch === epoch &&
    refreshCooldown.sid === sid &&
    refreshCooldown.until > Date.now()
  ) {
    return refreshCooldown.outcome
  }
  return null
}

function recordRefreshCooldown(
  epoch: number,
  sid: string | undefined,
  outcome: RefreshOutcome
): void {
  if (
    authEpoch === epoch &&
    outcome.kind === 'transient_error' &&
    isRateLimitedError(outcome.error)
  ) {
    const seconds = Math.max(1, rateLimitWaitSeconds(outcome.error) ?? 30)
    refreshCooldown = {
      epoch,
      sid,
      until: Date.now() + seconds * 1000,
      outcome,
    }
  }
}

function hasPendingTwoFactorFlow(): boolean {
  const auth = useAuthStore.getState().auth
  return Boolean(
    auth.pending2FAFlowToken &&
    (auth.pending2FAFlowExpiresAt === null ||
      auth.pending2FAFlowExpiresAt > Date.now() / 1000)
  )
}

/** Keep refresh and one protected read together against other Cookie writers. */
export function refreshAndRunAuthenticatedRead<T>(
  scope: { userId: number; sessionId: string },
  operation: (bundle: AuthBundle) => Promise<T>,
  signal?: AbortSignal
): Promise<
  | { kind: 'response'; response: T }
  | { kind: 'session'; outcome: RefreshOutcome }
> {
  return withAuthCookieLock(async (transaction) => {
    const assertScope = () => {
      const auth = useAuthStore.getState().auth
      if (
        auth.user?.id !== scope.userId ||
        auth.session?.sid !== scope.sessionId
      ) {
        throw new axios.CanceledError(
          'Authentication changed while waiting for read recovery'
        )
      }
    }
    assertScope()
    if (hasPendingTwoFactorFlow()) {
      return {
        kind: 'session',
        outcome: {
          kind: 'transient_error',
          error: new AuthRefreshSupersededError(),
        },
      }
    }
    const epoch = authEpoch
    const outcome =
      currentRefreshCooldown(epoch, scope.sessionId) ??
      (await runRefresh(epoch, transaction))
    recordRefreshCooldown(epoch, scope.sessionId, outcome)
    if (outcome.kind !== 'authenticated') return { kind: 'session', outcome }
    assertScope()
    // The Cookie response is known and committed. The read itself cannot
    // create Cookie uncertainty, even if it fails or its observer cancels.
    const response = await operation(outcome.bundle)
    assertScope()
    return { kind: 'response', response }
  }, signal)
}

export function refreshAuthentication(): Promise<RefreshOutcome> {
  if (hasPendingTwoFactorFlow()) {
    return Promise.resolve({
      kind: 'transient_error',
      error: new AuthRefreshSupersededError(),
    })
  }
  if (!refreshPromise) {
    const refreshEpoch = authEpoch
    const sid = useAuthStore.getState().auth.session?.sid
    const cooldown = currentRefreshCooldown(refreshEpoch, sid)
    if (cooldown) return Promise.resolve(cooldown)
    const accountWrite: RefreshAccountWrite = {}
    refreshAccountWrite = accountWrite
    refreshPromise = performRefreshWithBrowserLock(refreshEpoch, accountWrite)
      .then((outcome) => {
        recordRefreshCooldown(refreshEpoch, sid, outcome)
        return outcome
      })
      .finally(() => {
        refreshPromise = null
        refreshAccountWrite = null
      })
  }
  return refreshPromise
}

export type AuthWriteConfig = {
  skipAuthRefresh: true
  timeout: number
  authScope: { userId: number | undefined; sessionId: string | undefined }
}

/** Prepare outside the cookie lock; never replay a factor/password mutation. */
export function withAuthenticatedCookieWrite<T>(
  operation: (config: AuthWriteConfig) => Promise<T>,
  writesCookie = true
): Promise<T> {
  return withAuthCookieActivity(() =>
    performAuthenticatedCookieWrite(operation, writesCookie)
  )
}

async function performAuthenticatedCookieWrite<T>(
  operation: (config: AuthWriteConfig) => Promise<T>,
  writesCookie = true
): Promise<T> {
  const initial = useAuthStore.getState().auth
  const scope = { userId: initial.user?.id, sessionId: initial.session?.sid }
  const assertScope = () => {
    const auth = useAuthStore.getState().auth
    if (
      auth.user?.id !== scope.userId ||
      auth.session?.sid !== scope.sessionId
    ) {
      throw new axios.CanceledError('Authentication changed before operation')
    }
    return auth
  }
  if (
    initial.user &&
    initial.session &&
    (!initial.accessToken ||
      !initial.accessExpiresAt ||
      initial.accessExpiresAt <= Date.now() / 1000 + 60)
  ) {
    const outcome = await refreshAuthentication()
    assertScope()
    if (outcome.kind !== 'authenticated') {
      throw new Error(t('Session expired!'))
    }
  }
  const bearer = assertScope().accessToken
  try {
    return await withAuthCookieLock((transaction) => {
      if (assertScope().accessToken !== bearer) {
        throw new axios.CanceledError('Authentication changed while waiting')
      }
      const dispatch = () =>
        operation({
          skipAuthRefresh: true,
          timeout: AUTH_COOKIE_TIMEOUT_MS,
          authScope: scope,
        })
      return writesCookie ? transaction.request(dispatch) : dispatch()
    })
  } catch (error) {
    // A clearly rejected request may refresh the session only after the
    // callback has released the lock. The user must explicitly retry input.
    if (axios.isAxiosError(error) && error.response?.status === 401) {
      if (assertScope().accessToken === bearer) await refreshAuthentication()
    }
    throw error
  }
}

function currentValidAuthBundle(): AuthBundle | null {
  const auth = useAuthStore.getState().auth
  if (
    !auth.user ||
    !auth.accessToken ||
    !auth.accessExpiresAt ||
    !auth.session ||
    auth.accessExpiresAt <= Math.floor(Date.now() / 1000)
  ) {
    return null
  }
  return {
    access_token: auth.accessToken,
    token_type: 'Bearer',
    access_expires_at: auth.accessExpiresAt,
    user: auth.user,
    session: auth.session,
  }
}

export async function bootstrapAuthentication(): Promise<RefreshOutcome> {
  const bundle = currentValidAuthBundle()
  if (bundle) {
    useAuthStore.getState().auth.setBootstrapState('complete')
    return { kind: 'authenticated', bundle }
  }

  const auth = useAuthStore.getState().auth
  const hasStaleSession = Boolean(auth.user && auth.session)
  if (auth.bootstrapState === 'complete' && !hasStaleSession) {
    return { kind: 'anonymous' }
  }

  auth.setBootstrapState('checking')
  return refreshAuthentication()
}

export function getCommonHeaders(): Record<string, string> {
  const headers: Record<string, string> = {
    'Content-Type': 'application/json',
  }
  const accessToken = useAuthStore.getState().auth.accessToken
  if (accessToken) {
    headers.Authorization = `Bearer ${accessToken}`
  }
  return headers
}

export async function getFreshAuthHeaders(): Promise<Record<string, string>> {
  const auth = useAuthStore.getState().auth
  const refreshBefore = Math.floor(Date.now() / 1000) + 60
  if (
    auth.accessToken &&
    auth.accessExpiresAt &&
    auth.accessExpiresAt > refreshBefore
  ) {
    return getCommonHeaders()
  }

  const outcome = await refreshAuthentication()
  if (outcome.kind === 'authenticated') {
    return getCommonHeaders()
  }

  const current = useAuthStore.getState().auth
  if (
    current.accessToken &&
    current.accessExpiresAt &&
    current.accessExpiresAt > Math.floor(Date.now() / 1000)
  ) {
    return getCommonHeaders()
  }

  if (outcome.kind === 'transient_error') {
    throw new Error(t('Request failed'), { cause: outcome.error })
  }
  throw new Error(t('Session expired!'))
}
