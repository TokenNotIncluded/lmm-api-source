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
import { useTranslation } from 'react-i18next'

import { Skeleton } from '@/components/ui/skeleton'

import type { UserProfile } from '../types'
import { AccountBindingsTab } from './tabs/account-bindings-tab'
import { NotificationTab } from './tabs/notification-tab'

interface ProfileSettingsCardProps {
  profile: UserProfile | null
  loading: boolean
  onProfileUpdate: () => void | Promise<void>
}

export function ProfileSettingsCard({
  profile,
  loading,
  onProfileUpdate,
}: ProfileSettingsCardProps) {
  const { t } = useTranslation()
  if (loading) return <Skeleton className='h-56 w-full rounded-2xl' />
  return (
    <div className='space-y-8'>
      <section aria-labelledby='profile-bindings-title' className='space-y-5'>
        <header className='space-y-1'>
          <h2 id='profile-bindings-title' className='text-base font-semibold'>
            {t('Account Bindings')}
          </h2>
          <p className='text-muted-foreground text-sm'>
            {t('Manage the accounts and email you use to sign in.')}
          </p>
        </header>
        <AccountBindingsTab profile={profile} onUpdate={onProfileUpdate} />
      </section>
      <section
        aria-labelledby='profile-notifications-title'
        className='bg-card space-y-5 rounded-2xl p-5 sm:p-7'
      >
        <header className='space-y-1'>
          <h2
            id='profile-notifications-title'
            className='text-base font-semibold'
          >
            {t('Notifications')}
          </h2>
          <p className='text-muted-foreground text-sm'>
            {t('Set a reminder before your personal balance runs low.')}
          </p>
        </header>
        <NotificationTab
          profile={profile}
          onUpdate={() => void onProfileUpdate()}
        />
      </section>
    </div>
  )
}
