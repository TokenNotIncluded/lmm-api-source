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

import { getNativeSystemResources, listSystemInstances } from '../api'
import type { SystemInstance, SystemResourceSnapshot } from '../types'
import {
  HostResources,
  ProcessResources,
  ResourceMetric,
  ResourceSampleMetadata,
} from './resource-metrics'

const PHASE_LABELS = {
  preparing: 'Preparing',
  ready: 'Ready',
  draining: 'Draining',
  stopped: 'Stopped',
} as const

function InstanceRow({
  instance,
  snapshot,
}: {
  instance: SystemInstance
  snapshot?: SystemResourceSnapshot
}) {
  const { t } = useTranslation()
  const stopped = instance.phase === 'stopped'
  const metric = (label: string, value: string | number) => (
    <ResourceMetric label={label} value={value} />
  )
  return (
    <div
      role='row'
      data-instance-id={instance.reporter_id}
      className='grid gap-8 py-7 not-last:border-b @min-[48rem]/instances:grid-cols-3'
    >
      <div role='cell' className='min-w-0 space-y-5'>
        <div className='flex flex-wrap items-center justify-between gap-2'>
          <h4 className='text-sm font-semibold break-all'>
            {instance.node_name}
          </h4>
          <Badge
            variant={
              stopped
                ? 'secondary'
                : instance.status === 'online'
                  ? 'success'
                  : 'warning'
            }
          >
            {t(
              stopped
                ? 'Stopped'
                : instance.status === 'online'
                  ? 'Online'
                  : 'Stale'
            )}
          </Badge>
        </div>
        <dl className='space-y-3'>
          {metric(
            t('Status'),
            instance.phase ? t(PHASE_LABELS[instance.phase]) : t('Not reported')
          )}
          {metric(
            t('Version'),
            instance.info?.runtime?.version ?? t('Not reported')
          )}
          {metric(
            t('In-flight requests'),
            instance.in_flight ?? t('Not reported')
          )}
          {!stopped &&
            metric(
              t('Model readiness'),
              t(
                instance.status === 'online' && instance.models_ready
                  ? 'Ready'
                  : 'Unavailable'
              )
            )}
          {!stopped &&
            metric(
              t('RPC readiness'),
              t(
                instance.status === 'online' && instance.rpc_ready
                  ? 'Ready'
                  : 'Unavailable'
              )
            )}
          {metric(
            t('Started'),
            formatTimestampToDate(instance.started_at, 'seconds')
          )}
          {metric(
            t('Last heartbeat'),
            formatTimestampToDate(instance.last_seen_at, 'seconds')
          )}
        </dl>
        <details className='text-muted-foreground text-xs'>
          <summary className='focus-visible:outline-ring min-h-11 cursor-pointer py-3 focus-visible:outline-2'>
            {t('Instance ID')}
          </summary>
          <p className='font-mono break-all'>{instance.reporter_id}</p>
        </details>
      </div>
      <div role='cell' className='min-w-0 space-y-5'>
        <h4 className='text-sm font-medium @min-[48rem]/instances:hidden'>
          {t('System')}
        </h4>
        {snapshot ? (
          <>
            <HostResources snapshot={snapshot} />
            <ResourceSampleMetadata snapshot={snapshot} />
          </>
        ) : (
          <p className='text-muted-foreground text-xs'>{t('Not reported')}</p>
        )}
      </div>
      <div role='cell' className='min-w-0 space-y-7'>
        {snapshot ? (
          (['lmm-core', 'lmm-extensions'] as const).map((service) => {
            const process = snapshot.processes[service]
            return (
              <ProcessResources
                key={service}
                label={t(
                  service === 'lmm-core' ? 'Core process' : 'Extensions process'
                )}
                process={process}
              />
            )
          })
        ) : (
          <p className='text-muted-foreground text-xs'>{t('Not reported')}</p>
        )}
      </div>
    </div>
  )
}

export function SystemInstancesPanel() {
  const { t } = useTranslation()
  const session = useAuthStore((state) => state.auth.session)
  const query = useQuery({
    queryKey: ['system-info', 'instances', session?.sid],
    queryFn: listSystemInstances,
    refetchInterval: 10_000,
  })
  // Core deletes stopped boots; ignore any leftover historical rows client-side.
  const instances = (query.data?.data ?? []).filter(
    (instance) => instance.phase !== 'stopped'
  )
  const online = instances.filter((instance) => instance.status === 'online')
  // Resource snapshots have no boot ID. Never duplicate local samples across
  // remote instances, or guess an owner from a truncated heartbeat list.
  const resourceOwner =
    !query.isError && !query.data?.truncated && online.length === 1
      ? online[0].reporter_id
      : undefined
  const resources = useQuery({
    queryKey: ['system-info', 'native-resources', session?.sid, resourceOwner],
    queryFn: getNativeSystemResources,
    enabled: resourceOwner !== undefined,
    staleTime: 30_000,
    refetchInterval: 30_000,
    retry: false,
  })
  const rows = (items: SystemInstance[]) => (
    <div role='table' aria-label={t('Instances')}>
      <div
        role='row'
        className='text-muted-foreground hidden grid-cols-3 gap-8 border-b text-xs font-medium @min-[48rem]/instances:grid'
      >
        <div role='columnheader' className='py-3'>
          {t('Instances')}
        </div>
        <div role='columnheader' className='py-3'>
          {t('System')}
        </div>
        <div role='columnheader' className='py-3'>
          {t('Core process')} / {t('Extensions process')}
        </div>
      </div>
      {items.map((instance) => (
        <InstanceRow
          key={instance.reporter_id}
          instance={instance}
          snapshot={
            instance.reporter_id === resourceOwner && !resources.isError
              ? resources.data
              : undefined
          }
        />
      ))}
    </div>
  )
  return (
    <section className='@container/instances min-w-0 space-y-6'>
      <div className='flex flex-wrap items-start justify-between gap-4'>
        <div>
          <h3 className='text-sm font-semibold'>{t('Instances')}</h3>
          <p className='text-muted-foreground mt-1 text-xs'>
            {t(
              'System samples describe the host. Core and Extensions values describe their own processes.'
            )}
          </p>
        </div>
        <Button
          type='button'
          size='sm'
          variant='ghost'
          className='min-h-11'
          disabled={query.isFetching || resources.isFetching}
          onClick={() => {
            void query.refetch()
            if (resourceOwner !== undefined) void resources.refetch()
          }}
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
      ) : (
        <>
          {instances.length ? (
            rows(instances)
          ) : (
            <p className='text-muted-foreground p-4 text-sm'>
              {t('No instances have reported yet.')}
            </p>
          )}
          {resourceOwner !== undefined && resources.isLoading && (
            <p role='status' className='text-muted-foreground p-4 text-xs'>
              {t('Loading...')}
            </p>
          )}
          {resourceOwner !== undefined && resources.isError && (
            <ErrorState
              title={t('Failed to load data')}
              onRetry={() => void resources.refetch()}
              className='min-h-24'
            />
          )}
        </>
      )}
      {query.data?.truncated && (
        <p className='text-muted-foreground border-t p-3 text-xs'>
          {t('Showing the 200 most recent instance heartbeats.')}
        </p>
      )}
    </section>
  )
}
