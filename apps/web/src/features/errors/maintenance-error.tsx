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

import { Button } from '@/components/ui/button'

import { ErrorPageFrame } from './error-page-frame'

export function MaintenanceError() {
  const { t } = useTranslation()

  return (
    <ErrorPageFrame
      status='503'
      title={t('Temporarily unavailable')}
      description={t('Please try again in a moment.')}
      actions={
        <>
          <Button
            size='lg'
            className='error-editorial-action error-editorial-action-primary'
            onClick={() => window.location.reload()}
          >
            {t('Retry')}
          </Button>
          <Button
            variant='outline'
            className='error-editorial-action error-editorial-action-secondary'
            render={
              <a
                href='https://github.com/TokenNotIncluded/lmm-api'
                target='_blank'
                rel='noopener noreferrer'
              />
            }
          >
            {t('Learn more')}
          </Button>
        </>
      }
    />
  )
}
