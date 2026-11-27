/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.
*/
import { useTranslation } from 'react-i18next'

import { useFrontendUpdateAvailable } from '@/hooks/use-frontend-update-available'

import { Button } from './ui/button'

export function FrontendUpdateNotice() {
  const { t } = useTranslation()
  const available = useFrontendUpdateAvailable()
  if (!available) return null
  return (
    <div
      role='status'
      className='bg-muted/60 flex flex-wrap items-center justify-between gap-3 border-b px-4 py-2 text-sm'
    >
      <span>
        {t(
          'A newer interface is available. Refresh after finishing any payment.'
        )}
      </span>
      <Button
        type='button'
        size='sm'
        variant='outline'
        onClick={() => window.location.reload()}
      >
        {t('Refresh page')}
      </Button>
    </div>
  )
}
