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

import {
  sideDrawerContentClassName,
  sideDrawerHeaderClassName,
} from '@/components/drawer-layout'
import {
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
  SheetDescription,
} from '@/components/ui/sheet'
import { commerceScope } from '@/features/commerce/native-client'
import { ROLE } from '@/lib/roles'
import { useAuthStore } from '@/stores/auth-store'

import { NativeSubscriptionRecords } from '../../native-records'

interface Props {
  open: boolean
  onOpenChange: (open: boolean) => void
  user: { id: number; username?: string } | null
  onSuccess?: () => void
}

export function UserSubscriptionsDialog(props: Props) {
  const auth = useAuthStore((state) => state.auth)
  const { t } = useTranslation()
  const scope = commerceScope()
  return (
    <Sheet open={props.open} onOpenChange={props.onOpenChange}>
      <SheetContent className={sideDrawerContentClassName()}>
        <SheetHeader className={sideDrawerHeaderClassName()}>
          <SheetTitle>{t('Subscription records')}</SheetTitle>
          <SheetDescription>
            {props.user?.username || `#${props.user?.id ?? ''}`}
          </SheetDescription>
        </SheetHeader>
        {props.open &&
        props.user &&
        scope &&
        (auth.user?.role ?? 0) >= ROLE.ADMIN ? (
          <div className='min-h-0 overflow-y-auto px-4 pb-4'>
            <NativeSubscriptionRecords
              key={`${scope.userId}:${scope.sessionId}:${props.user.id}:${auth.user?.role}`}
              scope={scope}
              userId={props.user.id}
              onChanged={props.onSuccess}
            />
          </div>
        ) : props.open ? (
          <p className='p-4 text-sm'>{t('Access Forbidden')}</p>
        ) : null}
      </SheetContent>
    </Sheet>
  )
}
