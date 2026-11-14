/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.
*/
import { Link } from '@tanstack/react-router'
import { RotateCcw } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { ROLE } from '@/lib/roles'
import { useAuthStore } from '@/stores/auth-store'

import { NativeSubscriptionAdmin } from './native-admin'

export function Subscriptions() {
  const { t } = useTranslation()
  const role = useAuthStore((state) => state.auth.user?.role ?? 0)
  return (
    <div className='flex h-full min-h-0 flex-col'>
      {role >= ROLE.SUPER_ADMIN && (
        <div className='flex justify-end px-4 py-2'>
          <Button
            variant='outline'
            size='sm'
            render={<Link to='/subscriptions/reset' />}
          >
            <RotateCcw aria-hidden='true' />
            {t('Subscription reset workspace')}
          </Button>
        </div>
      )}
      <div className='min-h-0 flex-1'>
        <NativeSubscriptionAdmin />
      </div>
    </div>
  )
}
