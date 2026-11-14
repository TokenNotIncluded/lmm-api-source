/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import axios from 'axios'
import { useCallback, useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { useAuthStore } from '@/stores/auth-store'

import { logout, startTelegramLogin, telegramLogin } from '../api'
import {
  completeAuthResponse,
  hasCurrentLoginBundle,
  reportAuthError,
} from '../lib/auth-completion'
import {
  pickTelegramAuthorization,
  type TelegramLoginFlow,
} from '../lib/telegram-login'
import type { SystemStatus } from '../types'
import { useAuthRedirect } from './use-auth-redirect'

export function useTelegramLogin(
  status: SystemStatus | null,
  redirectTo?: string,
  onVerifiedAccountAppeal?: () => void
) {
  const { t } = useTranslation()
  const { handleLoginSuccess, redirectTo2FA } = useAuthRedirect()
  const [isLoading, setIsLoading] = useState(false)
  const [isTelegramDialogOpen, setIsTelegramDialogOpen] = useState(false)
  const [isTelegramPending, setIsTelegramPending] = useState(false)
  const telegramController = useRef<AbortController | null>(null)
  const telegramFlow = useRef<{
    flow: TelegramLoginFlow
    accountWriteEpoch: number
    submitted: boolean
  } | null>(null)
  useEffect(() => () => telegramController.current?.abort(), [])
  const handleTelegramLogin = async () => {
    if (telegramController.current) return
    if (!status?.telegram_bot_name?.trim()) {
      toast.error(t('Login failed'))
      return
    }

    const controller = new AbortController()
    telegramController.current = controller
    setIsLoading(true)
    try {
      const logoutResponse = await logout()
      if (!logoutResponse.success) {
        throw new Error(
          logoutResponse.message || t('Failed to sign out session')
        )
      }
      if (controller.signal.aborted) return
      const flow = await startTelegramLogin(controller.signal)
      if (
        controller.signal.aborted ||
        telegramController.current !== controller
      ) {
        return
      }
      telegramFlow.current = {
        flow,
        accountWriteEpoch: useAuthStore.getState().auth.accountWriteEpoch,
        submitted: false,
      }
      setIsTelegramDialogOpen(true)
    } catch (error: unknown) {
      if (telegramController.current === controller) {
        telegramController.current = null
        telegramFlow.current = null
      }
      if (controller.signal.aborted || axios.isCancel(error)) return
      showTelegramError(
        error,
        t('Failed to start {{provider}} login', { provider: 'Telegram' })
      )
    } finally {
      if (
        telegramController.current === controller ||
        !telegramController.current
      ) {
        setIsLoading(false)
      }
    }
  }

  const handleTelegramDialogChange = useCallback((open: boolean) => {
    setIsTelegramDialogOpen(open)
    if (!open) {
      telegramController.current?.abort()
      telegramController.current = null
      telegramFlow.current = null
      setIsTelegramPending(false)
      setIsLoading(false)
    }
  }, [])

  const showTelegramError = useCallback(
    (error: unknown, fallback = t('Login failed')) =>
      reportAuthError(error, t, fallback, onVerifiedAccountAppeal),
    [t, onVerifiedAccountAppeal]
  )

  const handleTelegramAuthorization = useCallback(
    async (value: unknown) => {
      const controller = telegramController.current
      const pending = telegramFlow.current
      if (
        !controller ||
        controller.signal.aborted ||
        !pending ||
        pending.submitted
      ) {
        return
      }
      if (
        useAuthStore.getState().auth.accountWriteEpoch !==
        pending.accountWriteEpoch
      ) {
        handleTelegramDialogChange(false)
        return
      }
      if (pending.flow.expires_at <= Date.now() / 1000) {
        handleTelegramDialogChange(false)
        toast.error(t('Login flow expired. Please sign in again.'))
        return
      }
      const authorization = pickTelegramAuthorization(value)
      if (!authorization) {
        handleTelegramDialogChange(false)
        toast.error(t('Login failed'))
        return
      }

      pending.submitted = true
      setIsTelegramPending(true)
      const isCurrent = () =>
        !controller.signal.aborted && telegramController.current === controller
      try {
        const response = await telegramLogin(
          authorization,
          pending.flow.flow_token,
          controller.signal
        )
        if (!isCurrent()) return
        setIsTelegramDialogOpen(false)
        telegramFlow.current = null
        const handled = await completeAuthResponse(response, {
          alreadyApplied: true,
          signal: controller.signal,
          onMFA: () => redirectTo2FA(redirectTo),
          onBundle: async (bundle) => {
            await handleLoginSuccess(
              bundle,
              redirectTo,
              true,
              controller.signal
            )
            if (isCurrent() && hasCurrentLoginBundle(bundle)) {
              toast.success(t('Welcome back!'))
            }
          },
        })
        if (!handled) showTelegramError(response)
      } catch (error: unknown) {
        if (!isCurrent() || axios.isCancel(error)) return
        setIsTelegramDialogOpen(false)
        showTelegramError(error)
      } finally {
        if (telegramController.current === controller) {
          telegramController.current = null
          telegramFlow.current = null
          setIsTelegramPending(false)
        }
      }
    },
    [
      handleLoginSuccess,
      redirectTo,
      redirectTo2FA,
      t,
      showTelegramError,
      handleTelegramDialogChange,
    ]
  )

  return {
    isLoading,
    isTelegramDialogOpen,
    isTelegramPending,
    handleTelegramLogin,
    handleTelegramAuthorization,
    handleTelegramDialogChange,
  }
}
