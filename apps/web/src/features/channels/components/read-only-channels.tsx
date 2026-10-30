/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useQuery } from '@tanstack/react-query'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { ErrorState } from '@/components/error-state'
import { LoadingState } from '@/components/loading-state'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { getServerErrorMessage } from '@/lib/server-error-message'
import { useAuthStore } from '@/stores/auth-store'

import {
  getChannels,
  searchChannels,
  setNativeChannelEnabled,
  removeNativeChannel,
} from '../api'
import { readNativeRoutingScope } from '../native-api'
import type { Channel } from '../types'

const PAGE_SIZE = 10

/** The routing snapshot supplies configuration facts, not legacy operational metrics. */
export function ReadOnlyChannels({
  controlsEnabled = false,
}: {
  controlsEnabled?: boolean
}) {
  const { t } = useTranslation()
  const auth = useAuthStore((state) => state.auth)
  const scope = readNativeRoutingScope(auth)
  const [input, setInput] = useState('')
  const [keyword, setKeyword] = useState('')
  const [page, setPage] = useState(1)
  const [pending, setPending] = useState<number | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [deleteId, setDeleteId] = useState<number | null>(null)
  // @context-fold begin read-only-channel-query
  const query = useQuery({
    queryKey: [
      'channels',
      'read-only',
      auth.user?.id,
      auth.session?.sid,
      auth.user?.role,
      keyword,
      page,
    ],
    queryFn: () =>
      keyword
        ? searchChannels({ keyword, p: page, page_size: PAGE_SIZE })
        : getChannels({ p: page, page_size: PAGE_SIZE }),
    retry: false,
  })
  const data = query.data?.data
  // @context-fold end read-only-channel-query
  const canWrite =
    controlsEnabled &&
    !!scope &&
    Number.isSafeInteger(data?.config_version) &&
    Number.isSafeInteger(data?.pricing_version)
  async function mutate(channel: Channel, remove: boolean) {
    if (
      !canWrite ||
      !Number.isSafeInteger(channel.id) ||
      channel.id <= 0 ||
      !data?.config_version ||
      !data.pricing_version ||
      pending !== null
    ) {
      return
    }
    setPending(channel.id)
    setError(null)
    const version = {
      routing: data.config_version,
      pricing: data.pricing_version,
    }
    try {
      if (remove) await removeNativeChannel(channel.id, version, scope)
      else {
        await setNativeChannelEnabled(
          channel.id,
          channel.status !== 1,
          version,
          scope
        )
      }
      setDeleteId(null)
      await query.refetch()
    } catch (cause) {
      setError(
        getServerErrorMessage(cause) ??
          (cause instanceof Error ? cause.message : t('Failed to save'))
      )
      await query.refetch()
    } finally {
      setPending(null)
    }
  }
  function actions(channel: Channel) {
    if (!canWrite || !Number.isSafeInteger(channel.id) || channel.id <= 0) {
      return null
    }
    return (
      <div className='flex flex-wrap gap-2'>
        <Button
          type='button'
          variant='outline'
          className='min-h-11'
          disabled={pending !== null}
          onClick={() => void mutate(channel, false)}
        >
          {channel.status === 1 ? t('Disable') : t('Enable')}
        </Button>
        {deleteId === channel.id ? (
          <>
            <Button
              type='button'
              variant='destructive'
              className='min-h-11'
              disabled={pending !== null}
              onClick={() => void mutate(channel, true)}
            >
              {t('Confirm delete')}
            </Button>
            <Button
              type='button'
              variant='ghost'
              className='min-h-11'
              disabled={pending !== null}
              onClick={() => setDeleteId(null)}
            >
              {t('Cancel')}
            </Button>
          </>
        ) : (
          <Button
            type='button'
            variant='ghost'
            className='min-h-11'
            disabled={pending !== null}
            onClick={() => setDeleteId(channel.id)}
          >
            {t('Delete')}
          </Button>
        )}
      </div>
    )
  }

  return (
    <div className='min-w-0 space-y-4'>
      {!controlsEnabled && (
        <Alert>
          <AlertTitle>{t('Read only')}</AlertTitle>
          <AlertDescription>
            {t(
              'Channels are read-only. Names and status come from the active routing configuration; balances and test results are unavailable.'
            )}
          </AlertDescription>
        </Alert>
      )}
      {error && (
        <Alert variant='destructive' role='alert'>
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      )}
      <form
        className='flex flex-col gap-2 sm:flex-row'
        onSubmit={(event) => {
          event.preventDefault()
          setKeyword(input.trim())
          setPage(1)
        }}
      >
        <Input
          className='min-h-11 min-w-0'
          value={input}
          onChange={(event) => setInput(event.target.value)}
          aria-label={t('Search')}
        />
        <Button type='submit' className='min-h-11 shrink-0'>
          {t('Search')}
        </Button>
      </form>
      {query.isLoading ? (
        <LoadingState />
      ) : query.isError ? (
        <ErrorState
          description={
            getServerErrorMessage(query.error) ?? t('Failed to load data')
          }
          onRetry={() => void query.refetch()}
        />
      ) : data ? (
        <>
          <p className='text-muted-foreground text-sm'>
            {t('Total')}: {data.total}
          </p>
          <div className='space-y-3 sm:hidden'>
            {data.items.map((channel) => (
              <dl key={channel.id} className='min-w-0 space-y-3 border p-3'>
                <div className='min-w-0'>
                  <dt className='text-muted-foreground text-xs'>{t('Name')}</dt>
                  <dd className='font-medium [overflow-wrap:anywhere]'>
                    {channel.name}
                  </dd>
                </div>
                <div className='grid min-w-0 grid-cols-2 gap-3'>
                  <div className='min-w-0'>
                    <dt className='text-muted-foreground text-xs'>{t('ID')}</dt>
                    <dd className='font-mono text-sm [overflow-wrap:anywhere]'>
                      {channel.id}
                    </dd>
                  </div>
                  <div className='min-w-0'>
                    <dt className='text-muted-foreground text-xs'>
                      {t('Status')}
                    </dt>
                    <dd className='text-sm [overflow-wrap:anywhere]'>
                      {channel.status === 1 ? t('Enabled') : t('Disabled')}
                    </dd>
                  </div>
                </div>
                <div className='min-w-0'>
                  <dt className='text-muted-foreground text-xs'>
                    {t('Group')}
                  </dt>
                  <dd className='text-sm [overflow-wrap:anywhere] whitespace-pre-wrap'>
                    {channel.group}
                  </dd>
                </div>
                <div className='min-w-0'>
                  <dt className='text-muted-foreground text-xs'>
                    {t('Models')}
                  </dt>
                  <dd className='text-sm [overflow-wrap:anywhere] whitespace-pre-wrap'>
                    {channel.models}
                  </dd>
                </div>
                {canWrite && (
                  <div>
                    <dt className='sr-only'>{t('Actions')}</dt>
                    <dd>{actions(channel)}</dd>
                  </div>
                )}
              </dl>
            ))}
          </div>
          <div className='hidden overflow-x-auto border sm:block'>
            <table className='w-full text-left text-sm'>
              <thead className='border-b'>
                <tr>
                  {['ID', 'Name', 'Status', 'Group', 'Models'].map((label) => (
                    <th key={label} className='p-3'>
                      {t(label)}
                    </th>
                  ))}
                  {canWrite && <th className='p-3'>{t('Actions')}</th>}
                </tr>
              </thead>
              <tbody className='divide-y'>
                {data.items.map((channel) => (
                  <tr key={channel.id}>
                    <td className='p-3'>{channel.id}</td>
                    <td className='p-3'>{channel.name}</td>
                    <td className='p-3'>
                      {channel.status === 1 ? t('Enabled') : t('Disabled')}
                    </td>
                    <td className='p-3'>{channel.group}</td>
                    <td className='max-w-md p-3 break-words whitespace-pre-wrap'>
                      {channel.models}
                    </td>
                    {canWrite && <td className='p-3'>{actions(channel)}</td>}
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          {data.items.length === 0 ? (
            <p className='text-muted-foreground border p-3'>
              {t('No data available')}
            </p>
          ) : null}
          <div className='flex flex-wrap items-center justify-end gap-3'>
            <Button
              type='button'
              variant='outline'
              className='min-h-11'
              disabled={page === 1}
              onClick={() => setPage(page - 1)}
            >
              {t('Previous')}
            </Button>
            <span className='text-sm'>
              {t('Page')} {page} /{' '}
              {Math.max(1, Math.ceil(data.total / PAGE_SIZE))}
            </span>
            <Button
              type='button'
              variant='outline'
              className='min-h-11'
              disabled={page * PAGE_SIZE >= data.total}
              onClick={() => setPage(page + 1)}
            >
              {t('Next')}
            </Button>
          </div>
        </>
      ) : null}
    </div>
  )
}
