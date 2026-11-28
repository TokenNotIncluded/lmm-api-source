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
import { Mail } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { useStatus } from '@/hooks/use-status'

import type { UserProfile } from '../../types'
import { BindingCard } from '../binding-card'
import { EmailBindDialog } from '../dialogs/email-bind-dialog'
import { NativeAccountLinks } from '../native-account-links'

type Props = { profile: UserProfile; onUpdate: () => void | Promise<void> }
function EmailAccountBinding({ profile, onUpdate }: Props) {
  const { t } = useTranslation(),
    [open, setOpen] = useState(false)
  return (
    <>
      <div className='grid grid-cols-1 gap-2.5 sm:grid-cols-2 sm:gap-3'>
        <BindingCard
          label={t('Email')}
          value={profile.email}
          bound={Boolean(profile.email)}
          showBadge={false}
          icon={<Mail className='h-4 w-4' />}
          action={{
            label: profile.email ? t('Change') : t('Bind'),
            onClick: () => setOpen(true),
          }}
        />
      </div>
      <EmailBindDialog
        key={profile.email}
        open={open}
        onOpenChange={setOpen}
        currentEmail={profile.email}
        onSuccess={onUpdate}
      />
    </>
  )
}
export function AccountBindingsTab({
  profile,
  onUpdate,
}: {
  profile: UserProfile | null
  onUpdate: () => void | Promise<void>
}) {
  const { t } = useTranslation()
  const { status, loading, capabilitiesReady } = useStatus()
  if (!profile || loading) return null
  return (
    <>
      {capabilitiesReady && status?.native_account_bindings === true ? (
        <NativeAccountLinks status={status} onUpdate={onUpdate} />
      ) : (
        <section aria-label={t('Linked sign-in accounts')} className='space-y-1'>
          <h3 className='text-sm font-medium'>{t('Linked sign-in accounts')}</h3>
          <p className='text-muted-foreground text-sm'>
            {capabilitiesReady && status?.native_account_bindings === false
              ? t('Disabled')
              : t('Account bindings are unavailable')}
          </p>
        </section>
      )}
      <EmailAccountBinding profile={profile} onUpdate={onUpdate} />
    </>
  )
}
