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
import { Link } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'

import { SettingsSection } from '../components/settings-section'

export function QuotaSettingsSection() {
  const { t } = useTranslation()
  const destinations = [
    {
      to: '/redemption-codes' as const,
      title: t('Promotion campaigns'),
      description: t(
        'Manage reward campaigns, budgets and redemption codes through the Core promotion service.'
      ),
    },
    {
      to: '/operations/sources' as const,
      title: t('Promotion links'),
      description: t(
        'Manage promotion links and review registration, payment and source reports.'
      ),
    },
    {
      to: '/system-settings/billing/$section' as const,
      params: { section: 'checkin' },
      title: t('Check-in Rewards'),
      description: t(
        'Set the daily check-in reward range and inspect the Core funding ceiling.'
      ),
    },
  ]
  return (
    <SettingsSection title={t('Quota Settings')}>
      <div className='max-w-3xl space-y-8'>
        <p className='text-muted-foreground text-sm'>
          {t(
            'Reward settings are managed by the services that apply them. Choose a supported workflow below.'
          )}
        </p>
        <div className='divide-y'>
          {destinations.map((destination) => (
            <div
              key={destination.to}
              className='flex flex-col gap-4 py-6 sm:flex-row sm:items-center sm:justify-between'
            >
              <div className='space-y-2'>
                <h3 className='font-medium'>{destination.title}</h3>
                <p className='text-muted-foreground max-w-xl text-sm'>
                  {destination.description}
                </p>
              </div>
              <Button
                render={
                  <Link to={destination.to} params={destination.params} />
                }
                variant='outline'
                className='shrink-0 self-start sm:self-auto'
              >
                {t('Open')}
              </Button>
            </div>
          ))}
        </div>
        <p className='text-muted-foreground text-sm'>
          {t(
            'Automatic registration and invitation rewards, referral thresholds and penalties have no online policy implementation in this deployment.'
          )}
        </p>
      </div>
    </SettingsSection>
  )
}
