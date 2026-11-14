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
import { useId, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import {
  IconDiscord,
  IconGithub,
  IconGoogle,
  IconLinuxDo,
  IconTelegram,
  IconWeChat,
} from '@/assets/brand-icons'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'

import { useOAuthLogin } from '../hooks/use-oauth-login'
import {
  federatedProviders,
  type FederatedProvider,
} from '../lib/federated-oauth'
import { isBuiltInGoogleConfigured } from '../lib/google-oauth'
import {
  isOAuthRegistrationAllowed,
  isOAuthProviderConfigured,
} from '../lib/registration'
import type { SystemStatus } from '../types'
import { TelegramLoginDialog } from './telegram-login-dialog'

type OAuthProvidersProps = {
  onVerifiedAccountAppeal?: () => void
  status: SystemStatus | null
  disabled?: boolean
  className?: string
  onWeChatLogin?: (provider: FederatedProvider) => void
  isWeChatLoading?: boolean
  redirectTo?: string
  acceptedLegal?: boolean
  featureGoogle?: boolean
  registrationOnly?: boolean
  registrationCapabilitiesReady: boolean
}

type ProviderButton = {
  key: string
  label: string
  onClick: () => void
  icon?: ReactNode
  disabled?: boolean
  shortLabel: string
  featured?: boolean
  registrationAllowed?: boolean
}

export function OAuthProviders({
  status,
  disabled = false,
  className,
  onWeChatLogin,
  isWeChatLoading = false,
  redirectTo,
  acceptedLegal = false,
  featureGoogle = false,
  registrationOnly = false,
  registrationCapabilitiesReady,
  onVerifiedAccountAppeal,
}: OAuthProvidersProps) {
  const { t } = useTranslation()
  const policyDescriptionId = useId()
  const {
    isLoading,
    githubButtonText,
    githubButtonDisabled,
    handleGoogleLogin,
    handleGitHubLogin,
    handleDiscordLogin,
    handleFederatedLogin,
    handleLinuxDOLogin,
    handleTelegramLogin,
    isTelegramDialogOpen,
    isTelegramPending,
    handleTelegramAuthorization,
    handleTelegramDialogChange,
  } = useOAuthLogin(status, redirectTo, acceptedLegal, onVerifiedAccountAppeal)

  const providerButtons: ProviderButton[] = []
  const registrationAllowed = (provider: string | FederatedProvider) =>
    !registrationOnly ||
    (registrationCapabilitiesReady &&
      isOAuthRegistrationAllowed(status, provider))

  const builtInGoogleAllowed = registrationAllowed('google')
  if (isBuiltInGoogleConfigured(status) && builtInGoogleAllowed) {
    providerButtons.push({
      key: 'google',
      label: t('Continue with Google'),
      shortLabel: 'Google',
      onClick: handleGoogleLogin,
      icon: <IconGoogle className='size-[18px]' />,
      featured: featureGoogle,
    })
  }

  if (
    isOAuthProviderConfigured(status, 'github') &&
    registrationAllowed('github')
  ) {
    providerButtons.push({
      key: 'github',
      label: githubButtonText || t('Continue with GitHub'),
      shortLabel: 'GitHub',
      onClick: handleGitHubLogin,
      icon: <IconGithub className='h-4 w-4' />,
      disabled: githubButtonDisabled,
    })
  }

  if (
    isOAuthProviderConfigured(status, 'discord') &&
    registrationAllowed('discord')
  ) {
    providerButtons.push({
      key: 'discord',
      label: t('Continue with Discord'),
      shortLabel: 'Discord',
      onClick: handleDiscordLogin,
      icon: <IconDiscord className='h-4 w-4' />,
    })
  }

  if (
    isOAuthProviderConfigured(status, 'linuxdo') &&
    registrationAllowed('linuxdo')
  ) {
    providerButtons.push({
      key: 'linuxdo',
      label: t('Continue with LinuxDO'),
      shortLabel: 'LinuxDO',
      onClick: handleLinuxDOLogin,
      icon: <IconLinuxDo className='h-4 w-4' />,
    })
  }

  if (
    isOAuthProviderConfigured(status, 'telegram') &&
    registrationAllowed('telegram')
  ) {
    providerButtons.push({
      key: 'telegram',
      label: t('Continue with Telegram'),
      shortLabel: 'Telegram',
      onClick: handleTelegramLogin,
      icon: <IconTelegram data-icon='inline-start' />,
    })
  }

  const nativeProviders = federatedProviders(status)
  for (const provider of nativeProviders) {
    if (!registrationAllowed(provider)) {
      continue
    }
    if (provider.kind === 'wechat' && !onWeChatLogin) continue
    providerButtons.push({
      key: `federated-${provider.id}`,
      registrationAllowed: isOAuthRegistrationAllowed(status, provider),
      label: t('Continue with {{name}}', { name: provider.name }),
      shortLabel: provider.name,
      onClick:
        provider.kind === 'wechat'
          ? () => onWeChatLogin?.(provider)
          : () => handleFederatedLogin(provider),
      icon:
        provider.kind === 'wechat' ? (
          <IconWeChat className='h-4 w-4' />
        ) : undefined,
      disabled: provider.kind === 'wechat' && isWeChatLoading,
    })
  }

  if (providerButtons.length === 0) return null

  for (const provider of providerButtons) {
    provider.registrationAllowed ??= isOAuthRegistrationAllowed(
      status,
      provider.key
    )
  }
  const hasLoginOnlyProvider =
    registrationCapabilitiesReady &&
    providerButtons.some((provider) => !provider.registrationAllowed)
  const featuredProvider = providerButtons.find((provider) => provider.featured)
  const otherProviders = providerButtons.filter(
    (provider) => !provider.featured
  )
  const showProviderDivider = !featuredProvider || otherProviders.length > 0

  const renderProviderButton = (provider: ProviderButton, compact: boolean) => (
    <Button
      key={provider.key}
      variant='outline'
      type='button'
      aria-label={provider.label}
      aria-describedby={
        registrationCapabilitiesReady && !provider.registrationAllowed
          ? policyDescriptionId
          : undefined
      }
      disabled={disabled || isLoading || provider.disabled}
      onClick={provider.onClick}
      className={cn(
        'h-auto w-full min-w-0 flex-col justify-center gap-1 rounded-xl py-2.5 shadow-none',
        compact ? 'min-h-14 px-2 text-sm' : 'min-h-16',
        compact &&
          'text-muted-foreground border-transparent bg-transparent hover:bg-muted/50 hover:text-foreground',
        provider.featured &&
          'border-[#747775] bg-white font-sans text-sm font-medium tracking-normal text-[#1f1f1f] hover:bg-[#f8faff] hover:text-[#1f1f1f] dark:border-[#8e918f] dark:bg-[#131314] dark:text-[#e3e3e3] dark:hover:bg-[#1f1f1f] dark:hover:text-white'
      )}
    >
      <span className='flex max-w-full items-center justify-center gap-2 text-center whitespace-normal'>
        {provider.icon}
        {provider.featured || !compact ? provider.label : provider.shortLabel}
      </span>
      {registrationCapabilitiesReady ? (
        <span className='text-xs font-normal opacity-75'>
          {provider.registrationAllowed
            ? t('Sign in or register')
            : t('Sign in only')}
        </span>
      ) : null}
    </Button>
  )

  return (
    <>
      <div className={cn('space-y-3', className)}>
        {featuredProvider
          ? renderProviderButton(featuredProvider, false)
          : null}

        {showProviderDivider ? (
          <div className='relative py-0.5' aria-hidden='true'>
            <div className='absolute inset-0 flex items-center'>
              <span className='w-full border-t' />
            </div>
            <div className='relative flex justify-center text-xs'>
              <span className='bg-background text-muted-foreground px-3'>
                {featuredProvider ? t('Or') : t('Or continue with')}
              </span>
            </div>
          </div>
        ) : null}

        {otherProviders.length > 0 ? (
          <div
            className={cn(
              featureGoogle ? 'grid grid-cols-2 gap-2' : 'flex flex-col gap-2'
            )}
          >
            {otherProviders.map((provider) =>
              renderProviderButton(provider, featureGoogle)
            )}
          </div>
        ) : null}
        {hasLoginOnlyProvider ? (
          <p id={policyDescriptionId} className='text-muted-foreground text-xs'>
            {t(
              'Sign-in-only providers require an account already linked to that provider.'
            )}
          </p>
        ) : null}
      </div>

      <TelegramLoginDialog
        open={isTelegramDialogOpen}
        botName={status?.telegram_bot_name ?? ''}
        pending={isTelegramPending}
        onOpenChange={handleTelegramDialogChange}
        onAuthorization={handleTelegramAuthorization}
      />
    </>
  )
}
