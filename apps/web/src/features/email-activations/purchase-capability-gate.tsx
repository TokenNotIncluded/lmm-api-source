/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { InformationCircleIcon } from '@hugeicons/core-free-icons'
import { HugeiconsIcon } from '@hugeicons/react'
import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import { LoadingState } from '@/components/loading-state'
import { Alert, AlertTitle } from '@/components/ui/alert'

import { useHeroSmsPurchaseCapability } from './purchase-capabilities'

export function HeroSmsPurchaseCapabilityGate({
  children,
}: {
  children: ReactNode
}) {
  const { t } = useTranslation()
  const capability = useHeroSmsPurchaseCapability()
  if (capability.pending) {
    return <LoadingState message={t('Loading...')} />
  }
  if (!capability.available) {
    return (
      <section
        className='mx-auto w-full max-w-5xl space-y-6 p-6'
        aria-label={t('Temporary activations')}
      >
        <h1 className='text-2xl font-semibold'>{t('Temporary activations')}</h1>
        <Alert>
          <HugeiconsIcon
            icon={InformationCircleIcon}
            strokeWidth={2}
            aria-hidden='true'
          />
          <AlertTitle>{t('Purchases unavailable')}</AlertTitle>
        </Alert>
      </section>
    )
  }
  return children
}
