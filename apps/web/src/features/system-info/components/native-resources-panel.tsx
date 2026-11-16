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
*/
import { useQuery } from '@tanstack/react-query'
import { RefreshCw, ServerCog } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { ErrorState } from '@/components/error-state'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { getServerErrorMessage } from '@/lib/server-error-message'
import { useAuthStore } from '@/stores/auth-store'

import { getNativeSystemResources } from '../api'
import {
  HostResources,
  ProcessResources,
  ResourceSampleMetadata,
} from './resource-metrics'

const POLL_INTERVAL_MS = 30_000

export function NativeResourcesPanel() {
  const { t } = useTranslation()
  const session = useAuthStore((state) => state.auth.session)
  const query = useQuery({
    queryKey: ['system-info', 'native-resources', session?.sid],
    queryFn: getNativeSystemResources,
    staleTime: POLL_INTERVAL_MS,
    refetchInterval: POLL_INTERVAL_MS,
    retry: false,
  })

  return (
    <section className='@container/resources space-y-6'>
      <div className='flex flex-wrap items-center justify-between gap-3'>
        <div className='flex min-w-0 items-center gap-2'>
          <ServerCog
            className='text-muted-foreground size-4 shrink-0'
            aria-hidden='true'
          />
          <h3 className='text-sm font-semibold'>
            {t('Service resource breakdown')}
          </h3>
        </div>
        <Button
          type='button'
          variant='ghost'
          size='sm'
          className='min-h-11'
          onClick={() => void query.refetch()}
          disabled={query.isFetching}
          aria-label={t('Refresh')}
        >
          <RefreshCw className='size-3.5' aria-hidden='true' />
          {query.isFetching ? t('Refreshing...') : t('Refresh')}
        </Button>
      </div>
      <div aria-busy={query.isFetching}>
        {query.isLoading ? (
          <div className='grid gap-3 sm:grid-cols-3'>
            {[0, 1, 2].map((key) => (
              <Skeleton key={key} className='h-28' />
            ))}
          </div>
        ) : query.isError ? (
          <ErrorState
            description={
              getServerErrorMessage(query.error) ?? t('Failed to load data')
            }
            onRetry={() => void query.refetch()}
            className='min-h-[220px]'
          />
        ) : query.data ? (
          <div className='space-y-6'>
            <p className='text-muted-foreground text-xs'>
              {t(
                'System samples describe the host. Core and Extensions values describe their own processes.'
              )}
            </p>
            <div className='grid gap-8 @min-[48rem]/resources:grid-cols-3'>
              <div className='min-w-0 space-y-5'>
                <HostResources snapshot={query.data} />
                <ResourceSampleMetadata snapshot={query.data} />
              </div>
              <ProcessResources
                label={t('Core process')}
                process={query.data.processes['lmm-core']}
              />
              <ProcessResources
                label={t('Extensions process')}
                process={query.data.processes['lmm-extensions']}
              />
            </div>
          </div>
        ) : null}
      </div>
    </section>
  )
}
