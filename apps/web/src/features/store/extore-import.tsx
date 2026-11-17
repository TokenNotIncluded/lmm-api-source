/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogTitle,
  DialogDescription,
} from '@/components/ui/dialog'

import { storeApi } from './api'
import { StoreExtoreResults } from './extore-results'
import type { ExtoreImportResult } from './extore-types'
import { StoreError, StoreLoading } from './shared'

export type { ExtoreImportResult } from './extore-types'

export function StoreExtoreImport({
  callbackUrl,
  onClose,
  onImported,
}: {
  callbackUrl: string | null
  onClose: () => void
  onImported: () => void
}) {
  const { t } = useTranslation()
  const [error, setError] = useState<unknown>(null)
  const [result, setResult] = useState<ExtoreImportResult | null>(null)
  const [busy, setBusy] = useState(true)
  const request = useRef<Promise<unknown> | null>(null)
  const notified = useRef(false)
  const [attempt, setAttempt] = useState(0)
  const [reconnecting, setReconnecting] = useState(false)
  useEffect(() => {
    let active = true
    setBusy(true)
    setError(null)
    if (callbackUrl && !reconnecting) {
      request.current ??= storeApi.extoreCatalog(callbackUrl)
      void request.current
        .then((value) => {
          if (!active) return
          const imported = value as ExtoreImportResult
          if (!Array.isArray(imported.results) || !imported.connection?.id) {
            throw new Error('Invalid Extore synchronization response')
          }
          setResult(imported)
          if (!notified.current) {
            notified.current = true
            onImported()
          }
        })
        .catch((issue: unknown) => {
          if (active) setError(issue)
        })
        .finally(() => {
          if (active) setBusy(false)
        })
    } else {
      request.current ??= storeApi.extoreAuthorize('', '')
      void request.current
        .then((value) => {
          if (!active) return
          const authorization = value as {
            authorization_url: string
            base_url: string
            redirect_uri: string
          }
          const url = new URL(authorization.authorization_url)
          if (
            url.origin !== authorization.base_url ||
            url.protocol !== 'https:' ||
            url.username ||
            url.password ||
            url.hash ||
            url.pathname !== '/oauth/authorize' ||
            authorization.redirect_uri !==
              `${window.location.origin}/store/manage`
          ) {
            throw new Error('Invalid Extore authorization response')
          }
          window.location.assign(url.href)
        })
        .catch((issue: unknown) => {
          if (active) {
            setError(issue)
            setBusy(false)
          }
        })
    }
    return () => {
      active = false
    }
  }, [callbackUrl, onImported, attempt, reconnecting])
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
    >
      <DialogContent className='max-h-[90svh] overflow-y-auto sm:max-w-xl'>
        <DialogTitle>{t('Import from Extore')}</DialogTitle>
        <DialogDescription>
          {t(
            'Approve products in Extore. All variants are imported automatically.'
          )}
        </DialogDescription>
        <StoreError error={error} />
        {busy ? (
          <StoreLoading />
        ) : result ? (
          <div className='space-y-4'>
            <StoreExtoreResults results={result.results} />
            <Button onClick={onClose}>{t('Back to store')}</Button>
          </div>
        ) : (
          <Button
            onClick={() => {
              request.current = null
              if (callbackUrl) setReconnecting(true)
              setAttempt((value) => value + 1)
            }}
          >
            {callbackUrl ? t('Connect again') : t('Try again')}
          </Button>
        )}
      </DialogContent>
    </Dialog>
  )
}
