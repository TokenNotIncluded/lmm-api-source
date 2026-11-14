/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { federatedProviders } from '@/features/auth/lib/federated-oauth'
import {
  isOAuthProviderConfigured,
  isOAuthRegistrationAllowed,
} from '@/features/auth/lib/registration'
import { useStatus } from '@/hooks/use-status'

export function OAuthRegistrationPolicy() {
  const { t } = useTranslation()
  const { status, capabilitiesReady, error, refetch } = useStatus()
  if (!capabilitiesReady) {
    return (
      <div role='status' className='space-y-2 text-sm'>
        <p>
          {error
            ? t('Unable to load registration settings')
            : t('Loading registration settings...')}
        </p>
        {error ? (
          <Button variant='outline' onClick={() => void refetch()}>
            {t('Retry')}
          </Button>
        ) : null}
      </div>
    )
  }
  const methods = [
    { id: 'google', name: 'Google' },
    { id: 'github', name: t('GitHub') },
    { id: 'discord', name: t('Discord') },
    { id: 'linuxdo', name: t('LinuxDO') },
    { id: 'telegram', name: t('Telegram') },
  ]
    .filter((provider) => isOAuthProviderConfigured(status, provider.id))
    .map((provider) => ({
      ...provider,
      registrationAllowed: isOAuthRegistrationAllowed(status, provider.id),
    }))
  methods.push(
    ...federatedProviders(status).map((provider) => ({
      id: `federated:${provider.id}`,
      name: provider.name,
      registrationAllowed: isOAuthRegistrationAllowed(status, provider),
    }))
  )
  return (
    <section className='space-y-3'>
      <h3 className='text-sm font-medium'>{t('Registration channels')}</h3>
      {methods.length ? (
        <dl className='divide-y rounded-md border'>
          {methods.map((method) => (
            <div
              key={method.id}
              className='flex flex-wrap items-center justify-between gap-2 px-3 py-3 text-sm'
            >
              <dt className='min-w-0 break-words'>{method.name}</dt>
              <dd className='text-muted-foreground'>
                {method.registrationAllowed
                  ? t('Sign in or register')
                  : t('Sign in only')}
              </dd>
            </div>
          ))}
        </dl>
      ) : (
        <p className='text-muted-foreground text-sm'>
          {t('No providers available')}
        </p>
      )}
    </section>
  )
}
