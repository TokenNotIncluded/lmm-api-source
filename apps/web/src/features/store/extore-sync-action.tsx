/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import { useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { toIntlLocale } from '@/i18n/languages'
import { useAuthStore } from '@/stores/auth-store'

import { storeApi } from './api'
import { extoreOrigin } from './extore-import-protocol'
import type { ExtoreConnection } from './extore-types'
import { StoreError } from './shared'

export function extoreSourceURL(
  connection: ExtoreConnection,
  sourceId?: string
) {
  if (!connection.origin) return undefined
  try {
    const origin = extoreOrigin(connection.origin)
    return `${origin}/admin${sourceId ? `?product=${encodeURIComponent(sourceId)}` : ''}`
  } catch {
    return undefined
  }
}

export function StoreExtoreSync({
  connectionId,
  sourceId,
  lastSyncedAt,
  disabled = false,
}: {
  connectionId: string
  sourceId: string
  lastSyncedAt?: number
  disabled?: boolean
}) {
  const { t, i18n } = useTranslation()
  const client = useQueryClient()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<unknown>(null)
  const [done, setDone] = useState(false)
  const owner = useAuthStore((state) => state.auth.user?.id)
  async function sync() {
    if (busy || disabled) return
    setBusy(true)
    setError(null)
    setDone(false)
    try {
      await storeApi.extoreSync(connectionId, sourceId)
      if (useAuthStore.getState().auth.user?.id !== owner) return
      setDone(true)
      await client.invalidateQueries({ queryKey: ['store'] })
    } catch (issue) {
      if (useAuthStore.getState().auth.user?.id === owner) setError(issue)
    } finally {
      setBusy(false)
    }
  }
  return (
    <div className='space-y-1'>
      <div className='flex flex-wrap items-center gap-2'>
        <Button
          size='sm'
          variant='secondary'
          disabled={busy || disabled}
          onClick={() => void sync()}
        >
          {busy ? t('Syncing...') : t('Sync')}
        </Button>
        {lastSyncedAt ? (
          <span className='text-muted-foreground text-xs'>
            {t('Last synced')}:{' '}
            {new Date(lastSyncedAt * 1000).toLocaleString(
              toIntlLocale(i18n.language)
            )}
          </span>
        ) : null}
        {done ? (
          <span role='status' className='text-success text-xs'>
            {t('Synced')}
          </span>
        ) : null}
      </div>
      <StoreError error={error} />
    </div>
  )
}
