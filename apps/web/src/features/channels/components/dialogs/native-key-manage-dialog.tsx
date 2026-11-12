/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Dialog } from '@/components/dialog'
import { LoadingState } from '@/components/loading-state'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { getServerErrorMessage } from '@/lib/server-error-message'

import {
  changeNativeKey,
  getNativeKeys,
  type NativeKeyList,
  type NativeKeyOperation,
  type NativeRoutingScope,
} from '../../native-api'

export function NativeKeyManageDialog({
  open,
  onOpenChange,
  scope,
  channelId,
  channelName,
  onChanged,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  scope: NativeRoutingScope
  channelId: number
  channelName: string
  onChanged?: () => void
}) {
  const { t } = useTranslation()
  const [keys, setKeys] = useState<NativeKeyList | null>(null)
  const [busy, setBusy] = useState(false)
  const pending = useRef(false)
  const [error, setError] = useState<string | null>(null)
  const [label, setLabel] = useState('')
  const [secret, setSecret] = useState('')
  const [confirmation, setConfirmation] = useState<string | null>(null)

  async function load() {
    setError(null)
    try {
      setKeys(await getNativeKeys(scope, channelId))
    } catch (cause) {
      setKeys(null)
      setError(
        getServerErrorMessage(cause) ??
          (cause instanceof Error ? cause.message : t('Failed to load'))
      )
    }
  }
  useEffect(() => {
    if (open) {
      setKeys(null)
      setLabel('')
      setSecret('')
      setConfirmation(null)
      void load()
    }
    // The parent keys this dialog by channel and administrative session.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, channelId])

  async function change(operation: NativeKeyOperation) {
    if (!keys || pending.current) return
    pending.current = true
    setBusy(true)
    setError(null)
    setConfirmation(null)
    try {
      await changeNativeKey(scope, channelId, keys.version, operation)
      setLabel('')
      setSecret('')
      setKeys(await getNativeKeys(scope, channelId))
      onChanged?.()
    } catch (cause) {
      setError(
        getServerErrorMessage(cause) ??
          (cause instanceof Error ? cause.message : t('Operation failed'))
      )
      try {
        setKeys(await getNativeKeys(scope, channelId))
      } catch {
        // Keep the mutation error visible when the recovery read also fails.
      }
    } finally {
      pending.current = false
      setBusy(false)
    }
  }

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t('Multi-Key Management')}
      description={`${channelName} · ${t('Manage multi-key status and configuration for this channel')}`}
      contentClassName='max-w-3xl'
      bodyClassName='space-y-4'
    >
      {error && (
        <Alert variant='destructive' role='alert'>
          <AlertDescription>
            <p>{error}</p>
            <Button
              type='button'
              variant='outline'
              className='mt-2 min-h-11'
              disabled={busy}
              onClick={() => void load()}
            >
              {t('Retry')}
            </Button>
          </AlertDescription>
        </Alert>
      )}
      {!keys ? (
        error ? null : (
          <LoadingState />
        )
      ) : (
        <>
          <div className='flex items-center gap-2'>
            <Checkbox
              id={`native-key-polling-${channelId}`}
              checked={keys.polling}
              disabled={busy}
              onCheckedChange={(checked) => {
                if (typeof checked === 'boolean') {
                  void change({ operation: 'set_polling', enabled: checked })
                }
              }}
            />
            <Label htmlFor={`native-key-polling-${channelId}`}>
              {t('Polling')}
            </Label>
          </div>
          <p className='text-muted-foreground text-sm'>
            {t('Active')}: {keys.active_reference.id} /{' '}
            {keys.active_reference.revision}
          </p>
          <div className='space-y-2'>
            {keys.keys.map((key) => {
              const active =
                key.reference.id === keys.active_reference.id &&
                key.reference.revision === keys.active_reference.revision
              const id = `key-${key.reference.id}-${key.reference.revision}`
              return (
                <div
                  key={id}
                  className='flex min-w-0 flex-wrap items-center justify-between gap-2 border p-3'
                >
                  <div className='min-w-0'>
                    <p className='[overflow-wrap:anywhere]'>{key.label}</p>
                    <p className='text-muted-foreground text-xs'>
                      {key.reference.id} / {key.reference.revision} ·{' '}
                      {key.enabled ? t('Enabled') : t('Disabled')}
                      {active && ` · ${t('Active')}`}
                    </p>
                  </div>
                  <div className='flex flex-wrap gap-2'>
                    <Button
                      type='button'
                      variant='outline'
                      className='min-h-11'
                      disabled={
                        busy || (active && (key.enabled || !keys.polling))
                      }
                      onClick={() =>
                        void change(
                          keys.polling
                            ? {
                                operation: 'set_enabled',
                                reference: key.reference,
                                enabled: !key.enabled,
                              }
                            : {
                                operation: 'activate',
                                reference: key.reference,
                              }
                        )
                      }
                    >
                      {keys.polling && key.enabled ? t('Disable') : t('Enable')}
                    </Button>
                    <Button
                      type='button'
                      variant={confirmation === id ? 'destructive' : 'outline'}
                      className='min-h-11'
                      disabled={busy || active}
                      onClick={() => {
                        if (confirmation === id) {
                          void change({
                            operation: 'remove',
                            reference: key.reference,
                          })
                        } else {
                          setConfirmation(id)
                        }
                      }}
                    >
                      {confirmation === id ? t('Confirm delete') : t('Delete')}
                    </Button>
                  </div>
                </div>
              )
            })}
          </div>
          <form
            className='grid gap-3 sm:grid-cols-2'
            onSubmit={(event) => {
              event.preventDefault()
              void change({
                operation: 'add',
                label,
                secret,
                activate: false,
              })
            }}
          >
            <label className='block space-y-2'>
              <span className='text-sm'>{t('Name')}</span>
              <Input
                className='min-h-11'
                value={label}
                maxLength={128}
                required
                disabled={busy}
                onChange={(event) => setLabel(event.target.value)}
              />
            </label>
            <label className='block space-y-2'>
              <span className='text-sm'>{t('API key')}</span>
              <Input
                className='min-h-11'
                type='password'
                autoComplete='off'
                value={secret}
                required
                disabled={busy}
                onChange={(event) => setSecret(event.target.value)}
              />
            </label>
            <Button
              type='submit'
              className='min-h-11 sm:col-span-2'
              disabled={busy}
            >
              {t('Add')}
            </Button>
          </form>
        </>
      )}
    </Dialog>
  )
}
