/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import { useQuery } from '@tanstack/react-query'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { useAuthStore } from '@/stores/auth-store'

import { storeApi } from './api'
import { StoreExtoreConnection } from './extore-connection'
import { extoreSourceURL } from './extore-import-protocol'
import { StoreExtoreSync } from './extore-sync-action'
import { StoreError, StoreLoading } from './shared'

export type { ExtoreConnections } from './extore-types'

function useConnections(enabled = true) {
  const id = useAuthStore((state) => state.auth.user?.id)
  return useQuery({
    queryKey: ['store', 'extore', id],
    queryFn: storeApi.extoreConnections,
    enabled: !!id && enabled,
    retry: false,
  })
}

export function StoreExtoreProductSync({ productId }: { productId: string }) {
  const { t } = useTranslation()
  const query = useConnections()
  const source = query.data?.sources.find(
    (item) => item.product_id === productId
  )
  if (!source) return null
  const connection = query.data?.connections.find(
    (item) => item.id === source.connection_id
  )
  const needsReconnect =
    !connection ||
    connection.status === 'expired' ||
    connection.status === 'reauthorization_required' ||
    connection.expires_at * 1000 <= Date.now()
  const href = connection && extoreSourceURL(connection, source.source_id)
  return (
    <div className='space-y-2'>
      <div className='text-muted-foreground flex flex-wrap items-center gap-2 text-xs'>
        <span>{t('Imported from Extore')}</span>
        {href && (
          <a
            className='text-primary underline-offset-4 hover:underline'
            href={href}
            target='_blank'
            rel='noopener noreferrer'
          >
            {t('Open Extore')}
          </a>
        )}
        {needsReconnect && <span>{t('Reconnect required')}</span>}
      </div>
      <StoreExtoreSync
        connectionId={source.connection_id}
        sourceId={source.source_id}
        lastSyncedAt={source.last_synced_at}
        disabled={needsReconnect}
      />
    </div>
  )
}

export function StoreExtoreSourceNotice({ productId }: { productId: string }) {
  const { t } = useTranslation()
  const query = useConnections()
  return query.data?.sources.some((item) => item.product_id === productId) ? (
    <p role='note' className='text-muted-foreground text-sm'>
      {t('Edit imported product settings in Extore, then sync.')}
    </p>
  ) : null
}

export function StoreExtoreConnections({
  enabled = true,
  onConnect,
}: {
  enabled?: boolean
  onConnect?: () => void
}) {
  const { t } = useTranslation()
  const query = useConnections(enabled)
  if (!enabled) return null
  return (
    <section className='space-y-4' aria-label={t('Extore workspace')}>
      <div className='flex flex-wrap items-center justify-between gap-3'>
        <div className='space-y-1'>
          <h2 className='text-base font-semibold'>{t('Extore workspace')}</h2>
          <p className='text-muted-foreground text-sm'>
            {t('Manage product details in Extore, then sync them here.')}
          </p>
        </div>
        {onConnect && (
          <Button variant='secondary' onClick={onConnect}>
            {t('Connect Extore')}
          </Button>
        )}
      </div>
      <StoreError error={query.error} retry={() => void query.refetch()} />
      {query.isPending ? (
        <StoreLoading />
      ) : (
        query.data?.connections.map((connection) => (
          <StoreExtoreConnection
            key={connection.id}
            connection={connection}
            sources={query.data.sources}
            onConnect={onConnect}
          />
        ))
      )}
    </section>
  )
}
