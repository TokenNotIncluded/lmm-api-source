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
import { useQuery } from '@tanstack/react-query'
import { RefreshCw } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { ErrorState } from '@/components/error-state'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { formatTimestampToDate } from '@/lib/format'
import { useAuthStore } from '@/stores/auth-store'

import { listSystemInstances } from '../api'

const PHASE_LABELS = {
  preparing: 'Preparing',
  ready: 'Ready',
  draining: 'Draining',
  stopped: 'Stopped',
} as const

export function SystemInstancesPanel() {
  const { t } = useTranslation()
  const session = useAuthStore((state) => state.auth.session)
  const query = useQuery({
    queryKey: ['system-info', 'instances', session?.sid],
    queryFn: listSystemInstances,
    refetchInterval: 10_000,
  })
  const instances = query.data?.data ?? []

  return (
    <section className='bg-card overflow-hidden border'>
      <div className='flex items-start justify-between gap-3 border-b p-4'>
        <div>
          <h3 className='text-sm font-semibold'>{t('Instances')}</h3>
          <p className='text-muted-foreground mt-1 text-xs'>
            {t(
              'Core heartbeats expire after {{seconds}} seconds. Each process has a separate boot ID.',
              {
                seconds: query.data?.stale_after_seconds ?? 45,
              }
            )}
          </p>
        </div>
        <Button
          type='button'
          size='sm'
          variant='outline'
          disabled={query.isFetching}
          onClick={() => void query.refetch()}
        >
          <RefreshCw className='size-3.5' aria-hidden='true' />
          {t('Refresh')}
        </Button>
      </div>
      {query.isLoading ? (
        <div className='p-4'>
          <Skeleton className='h-24 w-full' />
        </div>
      ) : query.isError ? (
        <ErrorState
          title={t('We could not load instances.')}
          onRetry={() => void query.refetch()}
          className='min-h-40'
        />
      ) : instances.length === 0 ? (
        <p className='text-muted-foreground p-4 text-sm'>
          {t('No instances have reported yet.')}
        </p>
      ) : (
        <ul className='divide-y'>
          {instances.map((instance) => (
            <li key={instance.reporter_id} className='space-y-3 p-4'>
              <div className='flex flex-wrap items-center justify-between gap-2'>
                <div className='min-w-0'>
                  <p className='text-sm font-medium break-all'>
                    {instance.node_name}
                  </p>
                  <p className='text-muted-foreground font-mono text-xs break-all'>
                    {instance.reporter_id}
                  </p>
                </div>
                <Badge
                  variant='secondary'
                  className={
                    instance.status === 'online'
                      ? 'console-status-success-badge'
                      : 'console-status-warning-badge'
                  }
                >
                  {t(instance.status === 'online' ? 'Online' : 'Stale')}
                </Badge>
              </div>
              <dl className='grid grid-cols-2 gap-3 text-xs lg:grid-cols-5'>
                <div>
                  <dt className='text-muted-foreground'>{t('Status')}</dt>
                  <dd>
                    {instance.phase
                      ? t(PHASE_LABELS[instance.phase])
                      : t('Not reported')}
                  </dd>
                </div>
                <div>
                  <dt className='text-muted-foreground'>{t('Version')}</dt>
                  <dd>{instance.info?.runtime?.version ?? '-'}</dd>
                </div>
                <div>
                  <dt className='text-muted-foreground'>
                    {t('In-flight requests')}
                  </dt>
                  <dd>{instance.in_flight ?? '-'}</dd>
                </div>
                <div>
                  <dt className='text-muted-foreground'>
                    {t('Model readiness')}
                  </dt>
                  <dd>
                    {t(
                      instance.status === 'online' && instance.models_ready
                        ? 'Ready'
                        : 'Unavailable'
                    )}
                  </dd>
                </div>
                <div>
                  <dt className='text-muted-foreground'>
                    {t('Last heartbeat')}
                  </dt>
                  <dd>
                    {formatTimestampToDate(instance.last_seen_at, 'seconds')}
                  </dd>
                </div>
              </dl>
            </li>
          ))}
        </ul>
      )}
      {query.data?.truncated && (
        <p className='text-muted-foreground border-t p-3 text-xs'>
          {t('Showing the 200 most recent instance heartbeats.')}
        </p>
      )}
    </section>
  )
}
