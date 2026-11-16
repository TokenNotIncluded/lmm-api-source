/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useQuery } from '@tanstack/react-query'
import { useState } from 'react'

import { LoadingState } from '@/components/loading-state'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { readNativeUsersScope } from '@/features/native-users/access'
import { toIntlLocale } from '@/i18n/languages'
import { useAuthStore } from '@/stores/auth-store'

import {
  readPricingSourceSnapshot,
  serializePricingSourceSnapshot,
  type PricingSourceSnapshot as Snapshot,
} from './pricing-source-snapshot-api'
import { usePricingSourceTranslation } from './pricing-source-snapshot-i18n'

function download(snapshot: Snapshot) {
  const url = URL.createObjectURL(
    new Blob([serializePricingSourceSnapshot(snapshot)], {
      type: 'application/json',
    })
  )
  const anchor = document.createElement('a')
  anchor.href = url
  anchor.download = `production-pricing-${snapshot.source_sha256.slice(0, 12)}.json`
  anchor.click()
  setTimeout(() => URL.revokeObjectURL(url), 0)
}

function SnapshotValues({ snapshot }: { snapshot: Snapshot }) {
  const { t, i18n } = usePricingSourceTranslation()
  const [search, setSearch] = useState('')
  const needle = search.trim().toLocaleLowerCase()
  const matches = (...values: string[]) =>
    values.some((value) => value.toLocaleLowerCase().includes(needle))
  const options = snapshot.options.filter((item) =>
    matches(item.key, item.value)
  )
  const unsupported = snapshot.unsupported.filter((item) =>
    matches(item.model, item.group, item.reason)
  )
  const channels = snapshot.enabled_channels.filter((item) =>
    matches(item.name, item.models, item.groups, item.model_mapping)
  )
  const locale = toIntlLocale(i18n.resolvedLanguage || i18n.language || 'en')
  return (
    <div className='space-y-6'>
      <dl className='grid gap-4 text-sm sm:grid-cols-2 lg:grid-cols-4'>
        <div>
          <dt className='text-muted-foreground'>{t('Captured at')}</dt>
          <dd className='mt-1'>
            <time
              dateTime={new Date(snapshot.observed_at * 1000).toISOString()}
            >
              {new Intl.DateTimeFormat(locale, {
                dateStyle: 'medium',
                timeStyle: 'short',
              }).format(snapshot.observed_at * 1000)}
            </time>
          </dd>
        </div>
        <div>
          <dt className='text-muted-foreground'>{t('Pricing options')}</dt>
          <dd className='mt-1 tabular-nums'>{snapshot.options.length}</dd>
        </div>
        <div>
          <dt className='text-muted-foreground'>
            {t('Enabled source channels')}
          </dt>
          <dd className='mt-1 tabular-nums'>
            {snapshot.enabled_channels.length}
          </dd>
        </div>
        <div>
          <dt className='text-muted-foreground'>
            {t('Unconverted model/group pairs')}
          </dt>
          <dd className='mt-1 tabular-nums'>{snapshot.unsupported.length}</dd>
        </div>
      </dl>
      <div className='flex flex-col gap-3 sm:flex-row'>
        <Input
          aria-label={t('Search original configuration')}
          placeholder={t('Search original configuration')}
          value={search}
          onChange={(event) => setSearch(event.target.value)}
          className='sm:max-w-md'
        />
        <Button
          type='button'
          variant='outline'
          onClick={() => download(snapshot)}
        >
          {t('Download complete JSON')}
        </Button>
      </div>
      {!options.length && !unsupported.length && !channels.length && (
        <p role='status' className='text-muted-foreground text-sm'>
          {t('No matching entries')}
        </p>
      )}
      <section className='space-y-3'>
        <h4 className='text-sm font-medium'>
          {t('Unconverted model/group pairs')} ({unsupported.length})
        </h4>
        <p className='text-muted-foreground text-sm'>
          {t(
            'These source entries were not converted by this import. Check the published native rules before use.'
          )}
        </p>
        <div className='max-h-72 space-y-2 overflow-auto'>
          {unsupported.map((item) => (
            <details
              key={JSON.stringify([item.model, item.group])}
              className='rounded-md border p-3 text-sm'
            >
              <summary className='cursor-pointer font-mono break-all'>
                {item.model} / {item.group}
              </summary>
              <p className='text-muted-foreground mt-3 break-all'>
                <span>{t('Conversion reason')}: </span>
                <code>{item.reason}</code>
              </p>
            </details>
          ))}
        </div>
      </section>
      <section className='space-y-3'>
        <h4 className='text-sm font-medium'>
          {t('Original pricing values')} ({options.length})
        </h4>
        {options.map((item) => (
          <details key={item.key} className='rounded-md border p-3 text-sm'>
            <summary className='cursor-pointer font-mono break-all'>
              {item.key}
            </summary>
            <pre className='bg-muted mt-3 max-h-80 overflow-auto rounded p-3 text-xs break-all whitespace-pre-wrap'>
              {item.value}
            </pre>
          </details>
        ))}
      </section>
      <details className='rounded-md border p-3 text-sm'>
        <summary className='cursor-pointer'>
          {t('Source model and group mappings')} ({channels.length})
        </summary>
        <div className='mt-3 max-h-80 space-y-3 overflow-auto'>
          {channels.map((channel) => (
            <pre
              key={channel.id}
              className='bg-muted rounded p-3 text-xs break-all whitespace-pre-wrap'
            >
              {JSON.stringify(channel, null, 2)}
            </pre>
          ))}
        </div>
      </details>
      <p className='text-muted-foreground text-xs break-all'>
        {t('Source SHA-256')}: <code>{snapshot.source_sha256}</code>
      </p>
    </div>
  )
}

export function PricingSourceSnapshot() {
  const { t } = usePricingSourceTranslation()
  const [open, setOpen] = useState(false)
  const auth = useAuthStore((state) => state.auth)
  const scope = readNativeUsersScope(auth)
  const query = useQuery({
    queryKey: [
      'pricing-source-snapshot',
      scope?.userId,
      scope?.sessionId,
      scope?.role,
    ],
    queryFn: ({ signal }) => {
      if (!scope) throw new Error('Administrator session required')
      return readPricingSourceSnapshot(scope, signal)
    },
    enabled: open && !!scope,
    retry: false,
    staleTime: 60_000,
    gcTime: 0,
    refetchOnWindowFocus: false,
  })
  if (!scope) return null
  return (
    <details
      className='rounded-lg border p-5'
      onToggle={(event) => setOpen(event.currentTarget.open)}
    >
      <summary className='cursor-pointer text-sm font-medium'>
        {t('Original configuration')}
      </summary>
      <div className='mt-5 space-y-6'>
        <p className='text-muted-foreground text-sm'>
          {t(
            'Read-only production snapshot. Only published native prices apply to requests.'
          )}
        </p>
        {open &&
          (query.isPending ? (
            <LoadingState />
          ) : query.isError ? (
            <div className='space-y-3'>
              <p role='alert' className='text-sm'>
                {t('The original pricing snapshot could not be loaded.')}
              </p>
              <Button
                type='button'
                variant='outline'
                disabled={query.isFetching}
                onClick={() => void query.refetch()}
              >
                {t('Retry')}
              </Button>
            </div>
          ) : query.data ? (
            <SnapshotValues snapshot={query.data} />
          ) : (
            <p role='status' className='text-muted-foreground text-sm'>
              {t('No original pricing snapshot is available.')}
            </p>
          ))}
      </div>
    </details>
  )
}
