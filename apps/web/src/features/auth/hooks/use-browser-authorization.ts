/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import axios from 'axios'
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { api } from '@/lib/api'
import {
  prepareLoginIntentVersion,
  type PreparedLoginIntent,
} from '@/lib/auth-session'

import { beginFederatedLogin } from '../federated-api'
import {
  reportAuthError,
  shouldSuppressAuthError,
} from '../lib/auth-completion'
import {
  EXTERNAL_BROWSER_PRESETS,
  buildExternalBrowserStartRequest,
  parseBrowserAuthorizationResponse,
  validateExternalBrowserAuthorizationUrl,
  type ExternalBrowserProvider,
} from '../lib/external-oauth'
import type { FederatedProvider } from '../lib/federated-oauth'
import {
  GOOGLE_BUILT_IN_PRESET,
  buildGoogleBrowserStartRequest,
  validateGoogleBrowserAuthorizationUrl,
} from '../lib/google-oauth'
import type { SystemStatus } from '../types'

const browserStartOptions = {
  skipAuthRefresh: true,
  skipErrorHandler: true,
  skipBusinessError: true,
}

type Strategy = {
  enabled?: boolean
  timeoutMs?: number
  onStart?: () => void
  onTimeout?: () => void
  onError?: () => void
  error: string
  authorize: (
    signal: AbortSignal,
    prepared?: PreparedLoginIntent
  ) => Promise<string>
}
type StrategyDefinition = Strategy
function authorizationStrategy(strategy: StrategyDefinition): Strategy {
  return {
    ...strategy,
    authorize: async (signal) => {
      if (signal.aborted) {
        throw new axios.CanceledError('Browser authorization cancelled')
      }
      const ready = await prepareLoginIntentVersion(signal)
      ready.assertCurrent()
      const url = await strategy.authorize(signal, ready)
      ready.assertCurrent()
      return url
    },
  }
}
export function useBrowserAuthorization(
  status: SystemStatus | null,
  acceptedLegal: boolean
) {
  const { t } = useTranslation()
  const providerErrors = {
    github: t('Failed to start GitHub login'),
    discord: t('Failed to start Discord login'),
    linuxdo: t('Failed to start LinuxDO login'),
  }
  const [isLoading, setIsLoading] = useState(false)
  const [githubPhase, setGithubPhase] = useState<
    'ready' | 'redirecting' | 'timeout'
  >('ready')
  const pending = useRef<AbortController | null>(null)
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null)
  useEffect(
    () => () => {
      pending.current?.abort()
      pending.current = null
      if (timer.current) clearTimeout(timer.current)
    },
    []
  )
  const run = async (strategy?: Strategy, invalidMessage?: string) => {
    if (pending.current || shouldSuppressAuthError()) return
    if (!strategy) {
      if (invalidMessage) toast.error(invalidMessage)
      return
    }
    if (strategy.enabled === false) return
    const controller = new AbortController()
    pending.current = controller
    setIsLoading(true)
    strategy.onStart?.()
    if (strategy.timeoutMs !== undefined) {
      timer.current = setTimeout(() => {
        controller.abort()
        setIsLoading(false)
        strategy.onTimeout?.()
      }, strategy.timeoutMs)
    }
    try {
      const url = await strategy.authorize(controller.signal)
      if (
        controller.signal.aborted ||
        pending.current !== controller ||
        shouldSuppressAuthError()
      ) {
        return
      }
      window.location.assign(url)
    } catch (error) {
      if (
        controller.signal.aborted ||
        pending.current !== controller ||
        shouldSuppressAuthError(error)
      ) {
        return
      }
      reportAuthError(error, t, strategy.error)
      strategy.onError?.()
    } finally {
      if (timer.current) clearTimeout(timer.current)
      if (pending.current === controller) {
        pending.current = null
        setIsLoading(false)
      }
    }
  }
  const start = (strategy?: StrategyDefinition, invalidMessage?: string) =>
    run(strategy ? authorizationStrategy(strategy) : undefined, invalidMessage)
  const fixedStrategy = (
    provider: ExternalBrowserProvider
  ): StrategyDefinition | undefined => {
    const clientId = status?.[`${provider}_client_id`]
    if (!clientId) return
    if (status?.external_oauth_browser === true) {
      const payload = buildExternalBrowserStartRequest(
        provider,
        status,
        acceptedLegal
      )
      return {
        error: providerErrors[provider],
        authorize: async () => {
          if (!payload) throw new Error('External browser login unavailable')
          const response = await api.post<unknown>(
            EXTERNAL_BROWSER_PRESETS[provider].browser_start,
            payload,
            browserStartOptions
          )
          return validateExternalBrowserAuthorizationUrl({
            provider,
            value: parseBrowserAuthorizationResponse(response.data),
            clientId,
            origin: window.location.origin,
            allowLocalDevelopment: import.meta.env.DEV,
          })
        },
      }
    }
    return undefined
  }
  const federatedStrategy = (
    provider: FederatedProvider
  ): StrategyDefinition | undefined =>
    provider.kind === 'wechat'
      ? undefined
      : {
          error: t('Failed to start {{provider}} login', {
            provider: provider.name,
          }),
          authorize: async (signal, ready) =>
            (await beginFederatedLogin(provider, acceptedLegal, signal, ready))
              .authorization_url,
        }
  const handleGoogleLogin = async () => {
    const payload = buildGoogleBrowserStartRequest(status, acceptedLegal)
    return start(
      payload
        ? {
            error: t('Failed to start {{provider}} login', {
              provider: 'Google',
            }),
            authorize: async () => {
              const response = await api.post<unknown>(
                GOOGLE_BUILT_IN_PRESET.browser_start,
                payload,
                browserStartOptions
              )
              return validateGoogleBrowserAuthorizationUrl(
                parseBrowserAuthorizationResponse(response.data),
                status?.google_client_id,
                window.location.origin,
                import.meta.env.DEV,
                status?.google_browser_callback
              )
            },
          }
        : undefined,
      t('Login failed')
    )
  }
  return {
    isLoading,
    githubButtonText:
      githubPhase === 'timeout'
        ? t('Request timed out, please refresh and restart GitHub login')
        : githubPhase === 'redirecting'
          ? t('Redirecting to GitHub...')
          : t('Continue with GitHub'),
    githubButtonDisabled: githubPhase !== 'ready',
    handleGoogleLogin,
    handleGitHubLogin: () => {
      const strategy = fixedStrategy('github')
      return start(
        strategy
          ? {
              ...strategy,
              enabled: githubPhase === 'ready',
              timeoutMs: 20000,
              onStart: () => setGithubPhase('redirecting'),
              onTimeout: () => setGithubPhase('timeout'),
              onError: () => setGithubPhase('ready'),
            }
          : undefined
      )
    },
    handleDiscordLogin: () => start(fixedStrategy('discord')),
    handleLinuxDOLogin: () => start(fixedStrategy('linuxdo')),
    handleFederatedLogin: (provider: FederatedProvider) =>
      start(federatedStrategy(provider)),
  }
}
