/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { toIntlLocale } from '@/i18n/languages'
import { useAuthStore } from '@/stores/auth-store'

import { storeApi } from './api'
import { extoreText } from './extore-import-protocol'
import { StoreExtoreResults } from './extore-results'
import { extoreSourceURL, StoreExtoreSync } from './extore-sync-action'
import type {
  ExtoreConnection,
  ExtoreSource,
  ExtoreSyncResult,
} from './extore-types'
import { StoreError, StoreLoading } from './shared'

export function StoreExtoreConnection({
  connection,
  sources,
  onConnect,
}: {
  connection: ExtoreConnection
  sources: ExtoreSource[]
  onConnect?: () => void
}) {
  const { t, i18n } = useTranslation()
  const client = useQueryClient()
  const owner = useAuthStore((state) => state.auth.user?.id)
  const [open, setOpen] = useState(false)
  const [search, setSearch] = useState('')
  const [busy, setBusy] = useState<'sync' | 'disconnect' | null>(null)
  const [disconnecting, setDisconnecting] = useState(false)
  const [error, setError] = useState<unknown>(null)
  const [results, setResults] = useState<ExtoreSyncResult[] | null>(null)
  const expired =
    connection.status === 'expired' ||
    connection.expires_at * 1000 <= Date.now()
  const needsReconnect =
    expired || connection.status === 'reauthorization_required'
  const query = useQuery({
    queryKey: ['store', 'extore-catalog', owner, connection.id],
    queryFn: () => storeApi.extoreConnectionCatalog(connection.id),
    enabled: open && !needsReconnect,
    retry: false,
  })
  const imported = sources.filter(
    (item) => item.connection_id === connection.id
  )
  const latestSync = Math.max(0, ...imported.map((item) => item.last_synced_at))
  const href = extoreSourceURL(connection)
  const products = query.data?.products.filter((product) => {
    const title = extoreText(product.product.name, i18n.language)
    return `${title} ${product.id}`
      .toLocaleLowerCase()
      .includes(search.toLocaleLowerCase().trim())
  })
  async function mutate(kind: 'sync' | 'disconnect') {
    if (busy) return
    setBusy(kind)
    setError(null)
    setResults(null)
    try {
      if (kind === 'disconnect') {
        await storeApi.extoreDisconnect(connection.id)
      } else {
        const value = await storeApi.extoreSyncAll(connection.id)
        if (useAuthStore.getState().auth.user?.id !== owner) return
        setResults(value.results)
      }
      if (useAuthStore.getState().auth.user?.id !== owner) return
      setDisconnecting(false)
      await client.invalidateQueries({ queryKey: ['store'] })
    } catch (issue) {
      if (useAuthStore.getState().auth.user?.id === owner) setError(issue)
    } finally {
      setBusy(null)
    }
  }
  return (
    <section
      className='bg-card space-y-5 rounded-2xl p-5 sm:p-6'
      aria-label={connection.shop_name}
    >
      <div className='flex flex-wrap items-start justify-between gap-4'>
        <div className='min-w-0 space-y-2'>
          <div className='flex flex-wrap items-center gap-2'>
            <h3 className='text-base font-semibold break-words'>
              {connection.shop_name}
            </h3>
            <span
              className={
                needsReconnect
                  ? 'bg-warning/10 text-warning rounded-full px-2.5 py-1 text-xs'
                  : 'bg-success/10 text-success rounded-full px-2.5 py-1 text-xs'
              }
            >
              {expired
                ? t('Expired')
                : needsReconnect
                  ? t('Reconnect required')
                  : t('Connected')}
            </span>
          </div>
          <p className='text-muted-foreground text-xs break-all'>
            {connection.origin}
          </p>
          <dl className='text-muted-foreground flex flex-wrap gap-x-6 gap-y-2 text-xs'>
            <div>
              <dt className='inline'>{t('Products')}: </dt>
              <dd className='text-foreground inline tabular-nums'>
                {imported.length}
              </dd>
            </div>
            <div>
              <dt className='inline'>{t('Connection expires')}: </dt>
              <dd className='inline'>
                {new Date(connection.expires_at * 1000).toLocaleDateString(
                  toIntlLocale(i18n.language)
                )}
              </dd>
            </div>
            {latestSync > 0 && (
              <div>
                <dt className='inline'>{t('Last synced')}: </dt>
                <dd className='inline'>
                  {new Date(latestSync * 1000).toLocaleString(
                    toIntlLocale(i18n.language)
                  )}
                </dd>
              </div>
            )}
          </dl>
        </div>
        <div className='flex flex-wrap items-center gap-2'>
          {needsReconnect ? (
            onConnect && (
              <Button size='sm' onClick={onConnect}>
                {t('Connect again')}
              </Button>
            )
          ) : (
            <Button
              size='sm'
              disabled={busy !== null}
              onClick={() => void mutate('sync')}
            >
              {busy === 'sync' ? t('Syncing...') : t('Sync all products')}
            </Button>
          )}
          {href && (
            <Button
              size='sm'
              variant='ghost'
              render={
                <a href={href} target='_blank' rel='noopener noreferrer' />
              }
            >
              {t('Open Extore')}
            </Button>
          )}
          {!needsReconnect && (
            <Button
              size='sm'
              variant='ghost'
              disabled={busy !== null}
              onClick={() => {
                setError(null)
                setDisconnecting(true)
              }}
            >
              {t('Disconnect')}
            </Button>
          )}
        </div>
      </div>
      {!disconnecting && <StoreError error={error} />}
      {results && <StoreExtoreResults results={results} />}
      {!needsReconnect && (
        <div className='space-y-4'>
          <Button
            size='sm'
            variant='ghost'
            aria-expanded={open}
            onClick={() => setOpen((value) => !value)}
          >
            {open ? t('Hide products') : t('View products')}
          </Button>
          {open && (
            <div className='space-y-4'>
              <Input
                value={search}
                onChange={(event) => setSearch(event.target.value)}
                placeholder={t('Search products')}
                aria-label={t('Search products')}
                className='max-w-sm'
              />
              <StoreError
                error={query.error}
                retry={() => void query.refetch()}
              />
              {query.isPending ? (
                <StoreLoading />
              ) : products?.length === 0 ? (
                <p className='text-muted-foreground py-3 text-sm'>
                  {t('No products in this connection.')}
                </p>
              ) : (
                <div className='divide-border/50 divide-y'>
                  {products?.map((product) => {
                    const source = imported.find(
                      (item) => item.source_id === product.id
                    )
                    const productURL = extoreSourceURL(connection, product.id)
                    return (
                      <article
                        key={product.id}
                        className='flex flex-wrap items-center justify-between gap-4 py-4'
                      >
                        <div className='min-w-0 space-y-1'>
                          <p className='font-medium break-words'>
                            {extoreText(product.product.name, i18n.language) ||
                              product.id}
                          </p>
                          <p className='text-muted-foreground text-xs'>
                            {t('Variants')}: {product.variants.length}
                          </p>
                          {product.card_summary && (
                            <p className='text-muted-foreground text-xs'>
                              {t('Extore code status')}:{' '}
                              {product.card_summary.total ?? 0} ·{' '}
                              {Object.entries(product.card_summary.states ?? {})
                                .filter(([, count]) => count > 0)
                                .map(
                                  ([status, count]) => `${t(status)}: ${count}`
                                )
                                .join(' · ')}
                            </p>
                          )}
                          {productURL && (
                            <a
                              className='text-primary text-xs underline-offset-4 hover:underline'
                              href={productURL}
                              target='_blank'
                              rel='noopener noreferrer'
                            >
                              {t('Open Extore')}
                            </a>
                          )}
                        </div>
                        <StoreExtoreSync
                          connectionId={connection.id}
                          sourceId={product.id}
                          lastSyncedAt={source?.last_synced_at}
                          disabled={busy !== null}
                        />
                      </article>
                    )
                  })}
                </div>
              )}
            </div>
          )}
        </div>
      )}
      <AlertDialog
        open={disconnecting}
        onOpenChange={(value) => {
          if (!busy) setDisconnecting(value)
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t('Disconnect Extore?')}</AlertDialogTitle>
            <AlertDialogDescription>
              {t(
                'Disconnecting stops future sync. Imported products and existing orders stay in your store.'
              )}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <StoreError error={error} />
          <AlertDialogFooter>
            <AlertDialogCancel disabled={busy !== null}>
              {t('Cancel')}
            </AlertDialogCancel>
            <AlertDialogAction
              variant='destructive'
              disabled={busy !== null}
              onClick={(event) => {
                event.preventDefault()
                void mutate('disconnect')
              }}
            >
              {busy === 'disconnect' ? t('Saving...') : t('Disconnect')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </section>
  )
}
