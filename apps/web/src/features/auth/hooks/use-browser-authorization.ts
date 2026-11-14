/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import axios from 'axios'
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { api } from '@/lib/api'
import {
  capturePasswordLoginGuard,
  prepareLoginIntentVersion,
  type PreparedLoginIntent,
} from '@/lib/auth-session'

import { createOAuthFlow, logout } from '../api'
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
import {
  federatedProviders,
  type FederatedProvider,
} from '../lib/federated-oauth'
import {
  GOOGLE_BUILT_IN_PRESET,
  buildGoogleBrowserStartRequest,
  validateGoogleBrowserAuthorizationUrl,
} from '../lib/google-oauth'
import {
  buildGitHubOAuthUrl,
  buildDiscordOAuthUrl,
  buildLinuxDOOAuthUrl,
  buildOIDCOAuthUrl,
} from '../lib/oauth'
import type { CustomOAuthProviderInfo, SystemStatus } from '../types'

const fixed = {
  github: {
    endpoint: 'https://github.com/login/oauth/authorize',
    build: buildGitHubOAuthUrl,
  },
  discord: {
    endpoint: 'https://discord.com/oauth2/authorize',
    build: buildDiscordOAuthUrl,
  },
  linuxdo: {
    endpoint: 'https://connect.linux.do/oauth2/authorize',
    build: buildLinuxDOOAuthUrl,
  },
} as const
const loopback = new Set(['localhost', '127.0.0.1', '::1', '[::1]'])
function legacyAuthorizationUrl(value: string, configured: string) {
  const url = new URL(value),
    endpoint = new URL(configured)
  for (const target of [url, endpoint]) {
    const local =
      import.meta.env.DEV &&
      target.protocol === 'http:' &&
      loopback.has(window.location.hostname) &&
      loopback.has(target.hostname)
    if (
      (target.protocol !== 'https:' && !local) ||
      !target.hostname ||
      target.username ||
      target.password ||
      target.hash
    ) {
      throw new Error('Untrusted OAuth authorization endpoint')
    }
  }
  if (url.origin !== endpoint.origin || url.pathname !== endpoint.pathname) {
    throw new Error('Untrusted OAuth authorization endpoint')
  }
  return url.toString()
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
type StrategyDefinition = Strategy & { resetSession?: boolean }
function authorizationStrategy({
  resetSession,
  ...strategy
}: StrategyDefinition): Strategy {
  return {
    ...strategy,
    authorize: async (signal) => {
      if (resetSession) {
        const response = await logout()
        if (!response.success) throw new Error('Failed to sign out session')
      }
      if (signal.aborted) {
        throw new axios.CanceledError('Browser authorization cancelled')
      }
      // Legacy reset's own logout/capture lifecycle is a separate boundary.
      const ready = resetSession
        ? undefined
        : await prepareLoginIntentVersion(signal)
      const assertCurrent = ready?.assertCurrent ?? capturePasswordLoginGuard()
      assertCurrent()
      const url = await strategy.authorize(signal, ready)
      assertCurrent()
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
    const preset = fixed[provider]
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
            { skipAuthRefresh: true }
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
    return {
      error: providerErrors[provider],
      resetSession: true,
      authorize: async () => {
        const state = await createOAuthFlow(provider, 'login', acceptedLegal)
        return legacyAuthorizationUrl(
          preset.build(clientId, state),
          preset.endpoint
        )
      },
    }
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
                { skipAuthRefresh: true }
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
  const handleOIDCLogin = async () => {
    if (status?.native_federated_auth === true) {
      const provider = federatedProviders(status).find(
        (row) => row.kind === 'oidc'
      )
      return start(provider ? federatedStrategy(provider) : undefined)
    }
    const endpoint = status?.oidc_authorization_endpoint,
      clientId = status?.oidc_client_id
    return start(
      endpoint && clientId
        ? {
            error: t('Failed to start OIDC login'),
            resetSession: true,
            authorize: async () => {
              const state = await createOAuthFlow(
                'oidc',
                'login',
                acceptedLegal
              )
              return legacyAuthorizationUrl(
                buildOIDCOAuthUrl(endpoint, clientId, state),
                endpoint
              )
            },
          }
        : undefined
    )
  }
  const handleCustomOAuthLogin = async (provider: CustomOAuthProviderInfo) =>
    start(
      provider.authorization_endpoint && provider.client_id
        ? {
            error: t('Failed to start {{provider}} login', {
              provider: provider.name,
            }),
            resetSession: true,
            authorize: async () => {
              const state = await createOAuthFlow(
                provider.slug,
                'login',
                acceptedLegal
              )
              const url = new URL(provider.authorization_endpoint)
              url.searchParams.set('client_id', provider.client_id)
              url.searchParams.set(
                'redirect_uri',
                `${window.location.origin}/oauth/${provider.slug}`
              )
              url.searchParams.set('response_type', 'code')
              url.searchParams.set('state', state)
              if (provider.scopes) {
                url.searchParams.set('scope', provider.scopes)
              }
              return legacyAuthorizationUrl(
                url.toString(),
                provider.authorization_endpoint
              )
            },
          }
        : undefined
    )
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
    handleOIDCLogin,
    handleFederatedLogin: (provider: FederatedProvider) =>
      start(federatedStrategy(provider)),
    handleCustomOAuthLogin,
  }
}
