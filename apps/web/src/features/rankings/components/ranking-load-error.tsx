/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { isAxiosError } from 'axios'
import type { TFunction } from 'i18next'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'

import { RankingsResponseError } from '../api'

function reason(t: TFunction, error: unknown) {
  const response = isAxiosError(error) ? error.response : undefined
  const payload: unknown = response?.data
  let code: unknown =
    error instanceof RankingsResponseError ? error.code : undefined
  if (payload && typeof payload === 'object') {
    if ('code' in payload) code = payload.code
    else if (
      'error' in payload &&
      payload.error &&
      typeof payload.error === 'object' &&
      'code' in payload.error
    ) {
      code = payload.error.code
    }
  }
  switch (code) {
    case 'RANKINGS_UNAVAILABLE':
      return t('The rankings service could not read usage data.')
    case 'RANKINGS_INVALID_PERIOD':
      return t('Choose another ranking period.')
    case 'EXTENSION_BUSY':
      return t('The rankings service is busy. Try again shortly.')
    case 'EXTENSION_NOT_ACCEPTING':
    case 'COMMUNITY_UNAVAILABLE':
    case 'core_migration_incomplete':
      return t(
        'The service is temporarily unavailable. Please try again later.'
      )
    default:
      return response?.status
        ? t('The rankings request failed (HTTP {{status}}).', {
            status: response.status,
          })
        : t('Unable to load rankings data')
  }
}

export function RankingLoadError({
  title,
  error,
  onRetry,
  retrying,
}: {
  title: string
  error: unknown
  onRetry: () => void
  retrying: boolean
}) {
  const { t } = useTranslation()
  return (
    <section
      aria-label={title}
      className='border-foreground/20 space-y-2 border-t px-4 py-6 text-center'
    >
      <h3 className='font-medium'>{title}</h3>
      <p role='status' className='text-muted-foreground text-sm'>
        {reason(t, error)}
      </p>
      <Button
        type='button'
        variant='outline'
        className='mt-2 min-h-11'
        disabled={retrying}
        onClick={onRetry}
      >
        {t('Retry')}
      </Button>
    </section>
  )
}
