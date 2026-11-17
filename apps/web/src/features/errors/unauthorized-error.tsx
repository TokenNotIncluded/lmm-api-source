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
import { useNavigate } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { useAuthStore } from '@/stores/auth-store'

import { ErrorPageFrame } from './error-page-frame'

export function UnauthorisedError() {
  const { t } = useTranslation()
  const navigate = useNavigate()
  const user = useAuthStore((state) => state.auth.user)

  return (
    <ErrorPageFrame
      status='401'
      title={t('Sign in to continue')}
      description={t('Sign in to your account to open this page.')}
      note={
        user
          ? t('Your session may have expired. Sign in again to continue.')
          : undefined
      }
      actions={
        <>
          <Button
            size='lg'
            className='error-editorial-action error-editorial-action-primary'
            onClick={() =>
              navigate({ to: '/sign-in', search: { redirect: '/' } })
            }
          >
            {t('Sign in to get started')}
          </Button>
          <Button
            variant='outline'
            className='error-editorial-action error-editorial-action-secondary'
            onClick={() => navigate({ to: '/' })}
          >
            {t('Back to Home')}
          </Button>
        </>
      }
    />
  )
}
