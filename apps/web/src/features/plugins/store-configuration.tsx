/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useQuery } from '@tanstack/react-query'
import { useTranslation } from 'react-i18next'

import { ErrorState } from '@/components/error-state'
import { storeApi } from '@/features/store/api'
import { StoreAnnouncementSettings } from '@/features/store/merchant-home'

export function StoreConfiguration() {
  const { t } = useTranslation()
  const config = useQuery({
    queryKey: ['store', 'config'],
    queryFn: storeApi.config,
    retry: false,
  })
  if (config.isPending) return <p role='status'>{t('Loading settings...')}</p>
  if (config.isError || !config.data.store_merchant_home_supported) {
    return (
      <ErrorState
        title={t('Unable to load settings')}
        onRetry={() => void config.refetch()}
      />
    )
  }
  return <StoreAnnouncementSettings supported />
}
