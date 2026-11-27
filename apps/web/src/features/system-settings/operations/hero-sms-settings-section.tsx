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
import { useMutation, useQuery } from '@tanstack/react-query'
import { useTranslation } from 'react-i18next'

import { ErrorState } from '@/components/error-state'
import { LoadingState } from '@/components/loading-state'
import { Button } from '@/components/ui/button'

import { useHeroSmsTranslations } from '../../email-activations/use-hero-sms-translations.js'
import { getHeroSmsSettings, testHeroSmsConnection } from './hero-sms-api'

export function HeroSmsSettingsSection() {
  const { t } = useTranslation()
  useHeroSmsTranslations()
  const settings = useQuery({
    queryKey: ['hero-sms-settings'],
    queryFn: getHeroSmsSettings,
  })
  const connection = useMutation({
    mutationFn: testHeroSmsConnection,
    retry: false,
  })

  if (settings.isError) {
    return (
      <ErrorState
        title={t('Unable to load HeroSMS settings')}
        onRetry={() => void settings.refetch()}
      />
    )
  }
  if (!settings.data) {
    return <LoadingState message={t('Loading HeroSMS settings...')} />
  }

  const { api_key_configured, pending_work, purchases_available } =
    settings.data

  return (
    <section
      aria-label={t('HeroSMS temporary activations')}
      className='flex flex-col items-start gap-4'
    >
      <dl className='grid gap-3'>
        <div>
          <dt className='text-sm font-medium'>
            {t('HeroSMS API key is managed on the host')}
          </dt>
          <dd className='text-muted-foreground text-sm'>
            {api_key_configured ? t('Configured') : t('Not configured')}
          </dd>
        </div>
        <div>
          <dt className='text-sm font-medium'>{t('Temporary activations')}</dt>
          <dd className='text-muted-foreground text-sm'>
            {purchases_available ? t('Available') : t('Purchases unavailable')}
          </dd>
        </div>
      </dl>
      {pending_work && <p>{t('Active orders are still being reconciled')}</p>}
      <Button
        type='button'
        size='sm'
        variant='outline'
        disabled={!api_key_configured || connection.isPending}
        onClick={() => connection.mutate()}
      >
        {t('Test connection')}
      </Button>
      {api_key_configured && connection.isSuccess && (
        <p role='status' className='text-sm'>
          {t(
            'The server can reach HeroSMS with the configured host credential.'
          )}
        </p>
      )}
      {api_key_configured && connection.isError && (
        <p role='alert' className='text-destructive text-sm'>
          {t('Connection test failed')}
        </p>
      )}
    </section>
  )
}
