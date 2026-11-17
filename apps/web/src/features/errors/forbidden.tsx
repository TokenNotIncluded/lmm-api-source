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
import { useNavigate, useRouter } from '@tanstack/react-router'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { useAuthStore } from '@/stores/auth-store'

import { ErrorPageFrame } from './error-page-frame'

export function ForbiddenError() {
  const { t } = useTranslation()
  const navigate = useNavigate()
  const { history } = useRouter()
  const user = useAuthStore((state) => state.auth.user)

  const signIn = () =>
    navigate({ to: '/sign-in', search: { redirect: '/wallet' } })

  return (
    <ErrorPageFrame
      status='403'
      title={t('Access denied')}
      description={t(
        'Your account does not have permission to open this page.'
      )}
      note={
        user
          ? t('Contact an administrator if you need access.')
          : t('Sign in to continue.')
      }
      actions={
        <>
          {user ? (
            <Button
              size='lg'
              className='error-editorial-action error-editorial-action-primary'
              onClick={() => navigate({ to: '/' })}
            >
              {t('Back to Home')}
            </Button>
          ) : (
            <Button
              size='lg'
              className='error-editorial-action error-editorial-action-primary'
              onClick={signIn}
            >
              {t('Sign in to get started')}
            </Button>
          )}
          {!user && (
            <Button
              variant='outline'
              className='error-editorial-action error-editorial-action-secondary'
              onClick={() => navigate({ to: '/' })}
            >
              {t('Back to Home')}
            </Button>
          )}
          <Button
            variant='outline'
            className='error-editorial-action error-editorial-action-secondary'
            onClick={() => history.go(-1)}
          >
            {t('Go Back')}
          </Button>
        </>
      }
    />
  )
}
