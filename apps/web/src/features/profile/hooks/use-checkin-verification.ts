/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useCallback, useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { isPasskeySupported, requestPasskeyAuthentication } from '@/lib/passkey'
import { useAuthStore } from '@/stores/auth-store'

import { checkinFailureCode, getCheckinChallenge, performCheckin } from '../api'
import type { CheckinChallenge, CheckinResponse } from '../checkin-schema'

interface Options {
  today: string
  pending: boolean
  userId: number | undefined
  sessionId: string | undefined
  onComplete: (result: CheckinResponse) => void
  onRefresh: () => Promise<{ today: string } | undefined>
}

export function useCheckinVerification({
  today,
  pending,
  userId,
  sessionId,
  onComplete,
  onRefresh,
}: Options) {
  const { t } = useTranslation()
  const operation = useRef<{
    day: string
    key: string
    submitted: boolean
  } | null>(null)
  const request = useRef<AbortController | null>(null)
  const challengeId = useRef<string | null>(null)
  const inFlight = useRef(false)
  const [loading, setLoading] = useState(false)
  const [challenge, setChallenge] = useState<CheckinChallenge | null>(null)
  const [token, setToken] = useState('')
  const [expired, setExpired] = useState(false)
  const [message, setMessage] = useState<string | null>(null)

  useEffect(
    () => () => {
      request.current?.abort()
      request.current = null
      challengeId.current = null
    },
    []
  )

  const current = (controller: AbortController) => {
    const auth = useAuthStore.getState().auth
    return (
      !controller.signal.aborted &&
      auth.user?.id === userId &&
      auth.session?.sid === sessionId
    )
  }

  const begin = () => {
    if (inFlight.current) return null
    inFlight.current = true
    setLoading(true)
    request.current?.abort()
    const controller = new AbortController()
    request.current = controller
    return controller
  }

  const finish = (controller: AbortController) => {
    if (request.current !== controller) return
    request.current = null
    inFlight.current = false
    setLoading(false)
  }

  const reset = () => {
    request.current?.abort()
    request.current = null
    inFlight.current = false
    challengeId.current = null
    setLoading(false)
    setChallenge(null)
    setToken('')
    setMessage(null)
    setExpired(false)
  }

  const failureMessage = (error: unknown, uncertain = false) => {
    switch (checkinFailureCode(error)) {
      case 'CHECKIN_VERIFICATION_REQUIRED':
        return t('Check-in verification expired. Please try again.')
      case 'CHECKIN_VERIFICATION_FAILED':
        return t('Check-in verification failed. Please try again.')
      case 'CHECKIN_PASSKEY_REQUIRED':
        return t('Set up a Passkey in Profile → Security before checking in.')
      case 'CHECKIN_RATE_LIMITED':
        return t('Too many check-in attempts. Please try again later.')
      case 'CHECKIN_UNAVAILABLE':
        return t('Check-in is temporarily unavailable. Please try again later.')
      default:
        return uncertain
          ? t(
              'Check-in result is not confirmed yet. Retry to recover the same request.'
            )
          : t('Check-in failed')
    }
  }

  const expire = useCallback(() => {
    request.current?.abort()
    request.current = null
    inFlight.current = false
    setLoading(false)
    setToken('')
    setExpired(true)
    setMessage(t('Check-in verification expired. Please try again.'))
    if (operation.current?.submitted) void onRefresh()
  }, [onRefresh, t])

  useEffect(() => {
    if (!challenge) return
    const timer = window.setTimeout(
      expire,
      Math.max(0, challenge.expires_at * 1000 - Date.now())
    )
    return () => window.clearTimeout(timer)
  }, [challenge, expire])

  const complete = (result: CheckinResponse) => {
    operation.current = null
    reset()
    onComplete(result)
  }

  const start = async () => {
    const controller = begin()
    if (!controller) return
    setMessage(null)
    const config = {
      signal: controller.signal,
      authScope: { userId, sessionId },
    }
    if (
      !operation.current ||
      (!operation.current.submitted && operation.current.day < today)
    ) {
      operation.current = {
        day: today,
        key: crypto.randomUUID(),
        submitted: false,
      }
    }
    try {
      // A lost response may have credited the account. Recover first, including
      // a server-persisted intention whose original browser key has been lost.
      if (operation.current.submitted || pending) {
        try {
          const response = await performCheckin(
            { operation_key: operation.current.key },
            config
          )
          if (!current(controller)) return
          if (!response.success) throw response
          complete(response.data)
          return
        } catch (error) {
          if (!current(controller)) return
          if (checkinFailureCode(error) !== 'CHECKIN_VERIFICATION_REQUIRED') {
            throw error
          }
          const status = await onRefresh()
          if (!current(controller)) return
          if (!status) throw error
          if (operation.current.day < status.today) {
            // The old day was checked for a receipt first. Today's reward is
            // a separate daily operation; it cannot reuse an old day's key.
            operation.current = {
              day: status.today,
              key: crypto.randomUUID(),
              submitted: false,
            }
          }
        }
      }
      const response = await getCheckinChallenge(operation.current.key, config)
      if (!current(controller)) return
      if (!response.success) throw response
      operation.current = {
        day: response.data.day,
        key: response.data.operation_key,
        submitted: false,
      }
      challengeId.current = response.data.challenge_id
      setChallenge(response.data)
      setToken('')
      setExpired(false)
    } catch (error) {
      if (!current(controller)) return
      toast.error(failureMessage(error, operation.current?.submitted))
      void onRefresh()
    } finally {
      finish(controller)
    }
  }

  const submit = async (turnstileToken: string) => {
    if (
      !challenge ||
      challengeId.current !== challenge.challenge_id ||
      !turnstileToken
    ) {
      return
    }
    if (expired || challenge.expires_at * 1000 <= Date.now()) {
      expire()
      return
    }
    const controller = begin()
    if (!controller) return
    setMessage(null)
    let submitted = false
    try {
      let assertion: Record<string, unknown> | undefined
      if (challenge.passkey_required) {
        const supported = await isPasskeySupported()
        if (!current(controller)) return
        if (!supported) {
          setMessage(t('Passkey is not available in this browser'))
          return
        }
        const result = await requestPasskeyAuthentication(
          challenge.passkey_options,
          controller.signal
        )
        if (!current(controller)) return
        if (!result) {
          setMessage(
            t('Passkey confirmation was cancelled. You can try again.')
          )
          return
        }
        assertion = result
      }
      if (!current(controller)) return
      if (challenge.expires_at * 1000 <= Date.now()) {
        expire()
        return
      }
      operation.current = {
        day: challenge.day,
        key: challenge.operation_key,
        submitted: true,
      }
      submitted = true
      const response = await performCheckin(
        {
          operation_key: challenge.operation_key,
          challenge_id: challenge.challenge_id,
          turnstile_token: turnstileToken,
          ...(assertion ? { passkey_assertion: assertion } : {}),
        },
        { signal: controller.signal, authScope: { userId, sessionId } }
      )
      if (!current(controller)) return
      if (!response.success) throw response
      complete(response.data)
    } catch (error) {
      if (!current(controller)) return
      if (
        !submitted &&
        error instanceof Error &&
        error.name === 'NotAllowedError'
      ) {
        setMessage(t('Passkey confirmation was cancelled. You can try again.'))
      } else {
        setToken('')
        setExpired(true)
        setMessage(failureMessage(error, submitted))
        void onRefresh()
      }
    } finally {
      finish(controller)
    }
  }

  return {
    loading,
    challenge,
    expired,
    message,
    tokenReady: token !== '',
    start,
    expire,
    close: () => {
      reset()
      if (operation.current?.submitted) void onRefresh()
    },
    verify: (value: string) => {
      const normalized = value.trim()
      if (!normalized || !challenge || expired) return
      setToken(normalized)
      if (!challenge.passkey_required) void submit(normalized)
    },
    confirmPasskey: () => submit(token),
  }
}
