/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import { useNavigate } from '@tanstack/react-router'
import {
  useCallback,
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from 'react'
import { useTranslation } from 'react-i18next'

import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import {
  authCookieCoordinationMessage,
  authCookieRecoveryMessage,
  authCookieStorageMessage,
  getAuthCookieBlockedState,
  subscribeAuthCookieState,
} from '@/lib/auth-cookie'
import {
  hasAuthCookieReconciliationHandoff,
  AuthCookieSessionNeedsRenewalError,
  AuthCookieSessionReauthenticationError,
  prepareAuthCookieReconciliation,
  reconcileAuthCookieSession,
  renewAuthCookieSession,
  restartAuthCookieAuthentication,
} from '@/lib/auth-cookie-reconciliation'
import {
  captureLocalAuthRecoveryScope,
  endLocalAuthSessionForRecovery,
  type LocalAuthRecoveryScope,
} from '@/lib/auth-session'
import { getAuthenticatedLandingRoute } from '@/lib/console-activation'
import { useAuthStore } from '@/stores/auth-store'

export function AuthCookieRecovery({
  expectedScope,
  onLocalEnd,
}: {
  expectedScope?: LocalAuthRecoveryScope
  onLocalEnd?: () => void
} = {}) {
  const { t } = useTranslation()
  const navigate = useNavigate()
  const auth = useAuthStore((store) => store.auth)
  const [result, setResult] = useState<'ended' | 'superseded' | null>(null)
  const [recovering, setRecovering] = useState(
    hasAuthCookieReconciliationHandoff
  )
  const [recoveryFailed, setRecoveryFailed] = useState(false)
  const [renewalRequired, setRenewalRequired] = useState(false)
  const [reauthenticationRequired, setReauthenticationRequired] =
    useState(false)
  const [startingSignIn, setStartingSignIn] = useState(false)
  const recoveryRequest = useRef<AbortController | null>(null)
  const state = useSyncExternalStore(
    subscribeAuthCookieState,
    getAuthCookieBlockedState,
    () => null
  )
  // Capture the displayed account, never a potentially newer account at click time.
  const scope = expectedScope ?? captureLocalAuthRecoveryScope()
  const ended =
    result === 'ended' &&
    !auth.user &&
    !auth.pending2FAFlowToken &&
    auth.bootstrapState === 'complete'
  const handleRecoveryError = useCallback((error: unknown) => {
    const needsRenewal = error instanceof AuthCookieSessionNeedsRenewalError
    const needsReauthentication =
      error instanceof AuthCookieSessionReauthenticationError
    setRenewalRequired(needsRenewal)
    setReauthenticationRequired(needsReauthentication)
    setRecoveryFailed(!needsRenewal && !needsReauthentication)
  }, [])
  const recover = useCallback(
    async (renew = false) => {
      if (recoveryRequest.current) return
      const controller = new AbortController()
      recoveryRequest.current = controller
      setRecovering(true)
      setRecoveryFailed(false)
      setRenewalRequired(false)
      setReauthenticationRequired(false)
      try {
        if (hasAuthCookieReconciliationHandoff()) {
          const bundle = await (
            renew ? renewAuthCookieSession : reconcileAuthCookieSession
          )(controller.signal)
          if (!controller.signal.aborted) {
            void navigate({
              to: getAuthenticatedLandingRoute(bundle.user),
              replace: true,
            })
          }
        } else {
          await prepareAuthCookieReconciliation(controller.signal)
        }
      } catch (error) {
        if (!controller.signal.aborted) handleRecoveryError(error)
      } finally {
        if (recoveryRequest.current === controller) {
          recoveryRequest.current = null
        }
        if (!controller.signal.aborted) setRecovering(false)
      }
    },
    [handleRecoveryError, navigate]
  )
  useEffect(() => {
    const controller = new AbortController()
    if (
      getAuthCookieBlockedState() === 'uncertain' &&
      hasAuthCookieReconciliationHandoff()
    ) {
      recoveryRequest.current = controller
      void reconcileAuthCookieSession(controller.signal)
        .then((bundle) => {
          if (!controller.signal.aborted) {
            void navigate({
              to: getAuthenticatedLandingRoute(bundle.user),
              replace: true,
            })
          }
        })
        .catch((error) => {
          if (!controller.signal.aborted) handleRecoveryError(error)
        })
        .finally(() => {
          if (recoveryRequest.current === controller) {
            recoveryRequest.current = null
          }
          if (!controller.signal.aborted) setRecovering(false)
        })
    }
    return () => {
      recoveryRequest.current?.abort()
      recoveryRequest.current = null
      controller.abort()
    }
  }, [handleRecoveryError, navigate])
  if (!state) return null
  const handleLocalEnd = () => {
    const outcome = endLocalAuthSessionForRecovery(scope)
    if (outcome.kind !== 'ended') {
      setResult('superseded')
      return
    }
    setResult('ended')
    onLocalEnd?.()
    void navigate({ to: '/', replace: true })
  }
  const handleReauthentication = async () => {
    if (startingSignIn) return
    setStartingSignIn(true)
    setRecoveryFailed(false)
    try {
      await restartAuthCookieAuthentication(scope)
      setResult('ended')
      onLocalEnd?.()
      window.location.replace('/sign-in')
    } catch {
      setStartingSignIn(false)
      setRecoveryFailed(true)
    }
  }
  return (
    <Alert
      variant={recoveryFailed ? 'destructive' : 'default'}
      className='relative z-50 rounded-none border-x-0 border-t-0 px-4 py-3 sm:px-6'
    >
      <AlertDescription className='mx-auto w-full max-w-7xl text-left [&_p:not(:last-child)]:mb-0'>
        <div className='flex min-w-0 flex-col gap-3 sm:flex-row sm:items-center sm:justify-between'>
          <div className='min-w-0 space-y-2'>
            <p className='text-foreground'>
              {t(
                state === 'uncertain'
                  ? authCookieRecoveryMessage
                  : state === 'coordination_unavailable'
                    ? authCookieCoordinationMessage
                    : authCookieStorageMessage
              )}
            </p>
            {reauthenticationRequired ? (
              <p role='status'>
                {t(
                  'This browser session is no longer available. Sign in again to continue.'
                )}
              </p>
            ) : null}
            {renewalRequired ? (
              <p role='status'>
                {t(
                  'Your browser session needs renewal before sign-in can be restored.'
                )}
              </p>
            ) : null}
            {recoveryFailed ? (
              <p role='status'>
                {t(
                  'The server could not confirm a current browser session. Recovery has not completed.'
                )}
              </p>
            ) : null}
          </div>
          {state === 'uncertain' ? (
            <div className='flex shrink-0 items-center'>
              {reauthenticationRequired ? (
                <Button
                  type='button'
                  disabled={startingSignIn}
                  aria-busy={startingSignIn}
                  className='h-auto min-h-11 whitespace-normal'
                  onClick={() => void handleReauthentication()}
                >
                  {t('Sign in again')}
                </Button>
              ) : (
                <Button
                  type='button'
                  variant='outline'
                  disabled={recovering}
                  aria-busy={recovering}
                  className='h-auto min-h-11 whitespace-normal'
                  onClick={() => void recover(renewalRequired)}
                >
                  {recovering
                    ? t('Checking browser session...')
                    : renewalRequired
                      ? t('Renew browser session')
                      : t('Recover browser session')}
                </Button>
              )}
            </div>
          ) : null}
        </div>
        {ended ? (
          <p role='status' className='mt-2'>
            {t(
              "This tab's local session has ended. Server sign-out is not confirmed."
            )}
          </p>
        ) : (
          <details
            className='mt-2 text-xs'
            open={result === 'superseded' || undefined}
          >
            <summary className='focus-visible:outline-ring w-fit cursor-pointer rounded-sm py-1 focus-visible:outline-2 focus-visible:outline-offset-4'>
              {t('Details')}
            </summary>
            <div className='mt-2 flex flex-wrap items-center gap-x-4 gap-y-2'>
              <p className='max-w-2xl'>
                {t(
                  'This only clears sign-in state in this tab. Server sign-out is not confirmed.'
                )}
              </p>
              <Button
                type='button'
                variant='ghost'
                className='h-auto min-h-11 px-2 text-xs whitespace-normal'
                onClick={handleLocalEnd}
              >
                {reauthenticationRequired
                  ? t('Back to Home')
                  : t("End this tab's local session")}
              </Button>
              {result === 'superseded' && (
                <p role='status' className='w-full'>
                  {t(
                    'Your session changed. Review the current account before trying again.'
                  )}
                </p>
              )}
            </div>
          </details>
        )}
      </AlertDescription>
    </Alert>
  )
}
