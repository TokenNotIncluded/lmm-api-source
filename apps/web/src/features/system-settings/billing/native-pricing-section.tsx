/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useTranslation } from 'react-i18next'

import { ErrorState } from '@/components/error-state'
import { LoadingState } from '@/components/loading-state'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { useRuntimeCapabilities } from '@/context/runtime-capabilities-context'
import { NativeRoutingManagement } from '@/features/channels/components/native-routing-management'
import { useChannelOps } from '@/features/channels/use-channel-ops'
import { useCapabilityFreshness } from '@/hooks/use-capability-freshness'
import { getServerErrorMessage } from '@/lib/server-error-message'

import { PricingSourceSnapshot } from './pricing-source-snapshot'

/** Prices belong to the published routing release, not the legacy options API. */
function NativePricingEditor() {
  const { t } = useTranslation()
  const runtime = useRuntimeCapabilities()
  const localQuery = useChannelOps(!runtime)
  const query = runtime?.channels ?? localQuery
  const phase = useCapabilityFreshness(query)

  if (query.isLoading) return <LoadingState />
  if (query.isError && !query.data) {
    return (
      <ErrorState
        description={
          getServerErrorMessage(query.error) ?? t('Failed to load data')
        }
        onRetry={() => void query.refetch()}
      />
    )
  }
  if (phase !== 'fresh') {
    return (
      <Alert role='status'>
        <AlertDescription className='space-y-3'>
          <p>
            {t(
              'Capability information is not current. Refresh before making changes.'
            )}
          </p>
          <Button
            type='button'
            variant='outline'
            disabled={query.isFetching}
            onClick={() => void query.refetch()}
          >
            {t('Refresh capabilities')}
          </Button>
        </AlertDescription>
      </Alert>
    )
  }
  if (query.data?.data?.native_configuration !== true) {
    return (
      <p className='text-muted-foreground text-sm' role='status'>
        {t('Capability unavailable')}
      </p>
    )
  }
  return <NativeRoutingManagement initialSection='prices' />
}

export function NativePricingSection() {
  return (
    <div className='space-y-8'>
      <NativePricingEditor />
      <PricingSourceSnapshot />
    </div>
  )
}
