/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { CanceledError } from 'axios'
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { capturePasswordLoginGuard } from '@/lib/auth-session'
import type { AuthBundle } from '@/stores/auth-store'

import { beginFederatedLogin, finishFederatedLogin } from '../federated-api'
import {
  completeAuthResponse,
  hasCurrentLoginBundle,
  reportAuthError,
  reportPasskeyCancellation,
  shouldSuppressAuthError,
  verifyAuthPasskey,
} from '../lib/auth-completion'
import {
  federatedProviders,
  forgetFederatedCeremony,
  type FederatedProvider,
} from '../lib/federated-oauth'
import type { SystemStatus } from '../types'
import { useAuthRedirect } from './use-auth-redirect'

export function useFederatedWechat(
  status: SystemStatus | null,
  acceptedLegal: boolean,
  requiresLegal: boolean,
  redirectTo?: string,
  onAppeal?: (value: unknown) => boolean | void
) {
  const { t } = useTranslation()
  const { handleLoginSuccess, redirectTo2FA, redirectToLogin } =
    useAuthRedirect()
  const [wechatCode, setWeChatCode] = useState('')
  const [isWeChatDialogOpen, setIsWeChatDialogOpen] = useState(false)
  const [isWeChatSubmitting, setIsWeChatSubmitting] = useState(false)
  const [wechatQrCodeUrl, setQr] = useState('')
  const flow = useRef<{
    provider: FederatedProvider
    state: string
    controller: AbortController
  } | null>(null)
  const pending = useRef(false)
  const generation = useRef(0)
  const cancel = () => {
    generation.current++
    flow.current?.controller.abort()
    if (flow.current) {
      forgetFederatedCeremony(window.sessionStorage, flow.current.state)
    }
    flow.current = null
    pending.current = false
  }
  useEffect(() => () => cancel(), [])
  const handleWeChatDialogChange = (open: boolean) => {
    setIsWeChatDialogOpen(open)
    if (!open) {
      cancel()
      setWeChatCode('')
      setIsWeChatSubmitting(false)
    }
  }
  const handleOpenWeChatDialog = async (selected?: FederatedProvider) => {
    if (pending.current || shouldSuppressAuthError()) return
    if (requiresLegal && !acceptedLegal) {
      toast.error(t('Please agree to the legal terms first'))
      return
    }
    pending.current = true
    setIsWeChatSubmitting(true)
    const attempt = ++generation.current
    const controller = new AbortController()
    try {
      const provider =
        selected ?? federatedProviders(status).find((p) => p.kind === 'wechat')
      if (!provider || provider.kind !== 'wechat') {
        throw new Error('Login failed')
      }
      flow.current = { provider, state: '', controller }
      const started = await beginFederatedLogin(
        provider,
        acceptedLegal,
        controller.signal
      )
      if (generation.current !== attempt) return
      flow.current.state = started.state
      setQr(provider.qr_code_url)
      setIsWeChatDialogOpen(true)
    } catch (error) {
      if (generation.current === attempt) {
        reportAuthError(error, t, t('Login failed'))
      }
    } finally {
      if (generation.current === attempt) {
        pending.current = false
        setIsWeChatSubmitting(false)
      }
    }
  }
  const handleWeChatLogin = async () => {
    if (shouldSuppressAuthError()) return
    const current = flow.current
    if (requiresLegal && !acceptedLegal) {
      toast.error(t('Please agree to the legal terms first'))
      return
    }
    if (!current || pending.current || !wechatCode.trim()) return
    pending.current = true
    setIsWeChatSubmitting(true)
    const attempt = generation.current
    try {
      const response = await finishFederatedLogin(
        current.provider.slug,
        { state: current.state, code: wechatCode.trim() },
        current.controller.signal
      )
      if (attempt !== generation.current) return
      const handlers = {
        alreadyApplied: true,
        signal: current.controller.signal,
        onMFA: () => {
          handleWeChatDialogChange(false)
          redirectTo2FA(redirectTo)
        },
        onBundle: async (bundle: AuthBundle) => {
          await handleLoginSuccess(
            bundle,
            redirectTo,
            true,
            current.controller.signal
          )
          if (
            attempt === generation.current &&
            hasCurrentLoginBundle(bundle) &&
            !shouldSuppressAuthError()
          ) {
            handleWeChatDialogChange(false)
            toast.success(t('Signed in via WeChat'))
          }
        },
      }
      const handled = await completeAuthResponse(response, {
        ...handlers,
        onPasskey: async (challenge) => {
          const assertLoginCurrent = capturePasswordLoginGuard()
          const result = await verifyAuthPasskey(challenge, () => {
            assertLoginCurrent()
            if (attempt !== generation.current) {
              throw new CanceledError('Login superseded')
            }
          })
          if (attempt !== generation.current) return
          if (reportPasskeyCancellation(result, t)) return
          if (!result || !(await completeAuthResponse(result, handlers))) {
            reportAuthError(
              result,
              t,
              t('Login failed'),
              onAppeal ?? (() => redirectToLogin(redirectTo))
            )
          }
        },
      })
      if (!handled) {
        reportAuthError(
          response,
          t,
          t('Login failed'),
          onAppeal ?? (() => redirectToLogin(redirectTo))
        )
      }
    } catch (error) {
      if (attempt !== generation.current) return
      if (reportPasskeyCancellation(error, t)) return
      reportAuthError(
        error,
        t,
        t('Login failed'),
        onAppeal ?? (() => redirectToLogin(redirectTo))
      )
    } finally {
      if (attempt === generation.current) {
        // Native state is one use even on provider errors. Reopen to start again.
        handleWeChatDialogChange(false)
      }
    }
  }
  return {
    wechatCode,
    setWeChatCode,
    isWeChatDialogOpen,
    isWeChatSubmitting,
    wechatQrCodeUrl,
    handleOpenWeChatDialog,
    handleWeChatDialogChange,
    handleWeChatLogin,
  }
}
