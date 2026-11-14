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

import { ErrorState } from '@/components/error-state'
import { SectionPageLayout } from '@/components/layout'
import { LoadingState } from '@/components/loading-state'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { useRuntimeCapabilities } from '@/context/runtime-capabilities-context'
import { useCapabilityFreshness } from '@/hooks/use-capability-freshness'
import { getServerErrorMessage } from '@/lib/server-error-message'

import { NativeRoutingToggle } from './components/native-routing-management'
import { ReadOnlyChannels } from './components/read-only-channels'
import { useChannelOps } from './use-channel-ops'

export function Channels() {
  const { t } = useTranslation()
  const runtime = useRuntimeCapabilities()
  const localQuery = useChannelOps(!runtime)
  const channelOpsQuery = runtime?.channels ?? localQuery
  const phase = useCapabilityFreshness(channelOpsQuery)
  return (
    <SectionPageLayout>
      <SectionPageLayout.Title>{t('Channels')}</SectionPageLayout.Title>
      <SectionPageLayout.Content>
        {channelOpsQuery.isLoading ? (
          <LoadingState />
        ) : channelOpsQuery.isError && !channelOpsQuery.data ? (
          <ErrorState
            description={
              getServerErrorMessage(channelOpsQuery.error) ??
              t('Failed to load data')
            }
            onRetry={() => void channelOpsQuery.refetch()}
          />
        ) : (
          <>
            {phase !== 'fresh' && (
              <Alert className='mb-4' role='status'>
                <AlertDescription>
                  <p>
                    {t(
                      'Capability information is not current. Refresh before making changes.'
                    )}
                  </p>
                  {channelOpsQuery.isError && (
                    <p>
                      {getServerErrorMessage(channelOpsQuery.error) ??
                        t('Failed to load data')}
                    </p>
                  )}
                  <Button
                    type='button'
                    variant='outline'
                    className='mt-2 min-h-11'
                    disabled={channelOpsQuery.isFetching}
                    onClick={() => void channelOpsQuery.refetch()}
                  >
                    {t('Refresh capabilities')}
                  </Button>
                </AlertDescription>
              </Alert>
            )}
            {phase === 'fresh' &&
              !channelOpsQuery.isError &&
              channelOpsQuery.data?.data?.native_configuration === true && (
                <NativeRoutingToggle />
              )}
            <ReadOnlyChannels
              controlsEnabled={
                phase === 'fresh' &&
                !channelOpsQuery.isError &&
                channelOpsQuery.data?.data?.native_control === true
              }
              keysEnabled={
                phase === 'fresh' &&
                !channelOpsQuery.isError &&
                channelOpsQuery.data?.data?.native_secrets === true
              }
            />
          </>
        )}
      </SectionPageLayout.Content>
    </SectionPageLayout>
  )
}
