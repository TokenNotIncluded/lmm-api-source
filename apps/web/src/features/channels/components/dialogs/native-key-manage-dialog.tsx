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
import { Textarea } from '@/components/ui/textarea'
import { getServerErrorMessage } from '@/lib/server-error-message'

import {
  changeNativeKey,
  getNativeKeys,
  verifyReplacementCredential,
  type NativeUpstream,
  type NativeKeyList,
  type NativeKeyOperation,
  type NativeRoutingScope,
} from '../../native-api'
import { NativeKeyRotation } from './native-key-rotation'

export function NativeKeyManageDialog({
  open,
  onOpenChange,
  scope,
  channelId,
  channelName,
  writable = true,
  bulkImportAllowed = false,
  rotationChannel,
  rotationVersion,
  draftKeyProbeAllowed = false,
  onChanged,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  scope: NativeRoutingScope
  channelId: number
  channelName: string
  writable?: boolean
  bulkImportAllowed?: boolean
  rotationChannel?: NativeUpstream
  rotationVersion?: number
  draftKeyProbeAllowed?: boolean
  onChanged?: () => void
}) {
  const { t } = useTranslation()
  const [keys, setKeys] = useState<NativeKeyList | null>(null)
  const [busy, setBusy] = useState(false)
  const pending = useRef(false)
  const [error, setError] = useState<string | null>(null)
  const [label, setLabel] = useState('')
  const [secret, setSecret] = useState('')
  const [batch, setBatch] = useState(false)
  const [batchSecrets, setBatchSecrets] = useState('')
  const [imported, setImported] = useState<number | null>(null)
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
    setLabel('')
    setSecret('')
    setBatchSecrets('')
    setBatch(false)
    setImported(null)
    setConfirmation(null)
    if (open) {
      setKeys(null)
      void load()
    }
    // The parent keys this dialog by channel and administrative session.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, channelId])
  useEffect(() => {
    if (!bulkImportAllowed) {
      setBatch(false)
      setBatchSecrets('')
      setImported(null)
    }
  }, [bulkImportAllowed])

  async function change(operation: NativeKeyOperation) {
    if (
      !writable ||
      !keys ||
      pending.current ||
      (operation.operation === 'add_many' && !bulkImportAllowed)
    ) {
      return
    }
    pending.current = true
    setBusy(true)
    setError(null)
    setImported(null)
    setConfirmation(null)
    // Submitted credentials never survive an uncertain mutation for blind replay.
    setSecret('')
    setBatchSecrets('')
    try {
      const receipt = await changeNativeKey(
        scope,
        channelId,
        keys.version,
        operation
      )
      setImported(
        operation.operation === 'add_many'
          ? (receipt.imported_count ?? null)
          : null
      )
      setLabel('')
      setSecret('')
      setBatchSecrets('')
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

  const canRotate =
    open &&
    writable &&
    draftKeyProbeAllowed &&
    rotationChannel?.id === channelId &&
    !!keys &&
    keys.version.routing === rotationVersion &&
    !keys.polling &&
    keys.active_reference.id === rotationChannel.credential.id &&
    keys.active_reference.revision === rotationChannel.credential.revision

  async function switchCredential(candidate: {
    label: string
    secret: string
  }) {
    if (!canRotate || !keys || pending.current) throw new Error('Unavailable')
    pending.current = true
    setBusy(true)
    setError(null)
    const failure = t('Check the active key before trying another switch.')
    let writeFailed = false
    try {
      try {
        await changeNativeKey(scope, channelId, keys.version, {
          operation: 'add',
          ...candidate,
          activate: true,
        })
      } catch {
        writeFailed = true
      }
      setKeys(await getNativeKeys(scope, channelId))
      if (writeFailed) throw new Error(failure)
    } catch {
      setError(failure)
      throw new Error(failure)
    } finally {
      pending.current = false
      setBusy(false)
      onChanged?.()
    }
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(nextOpen) => {
        if (!nextOpen) {
          setLabel('')
          setSecret('')
          setBatchSecrets('')
          setImported(null)
        }
        onOpenChange(nextOpen)
      }}
      title={t('Multi-Key Management')}
      description={`${channelName} · ${t('Manage multi-key status and configuration for this channel')}`}
      contentClassName='max-w-3xl'
      bodyClassName='space-y-4'
    >
      {!writable && (
        <Alert role='status'>
          <AlertDescription>
            {t(
              'Capability information is not current. Refresh before making changes.'
            )}
          </AlertDescription>
        </Alert>
      )}
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
              disabled={busy || !writable}
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
          {canRotate && rotationChannel ? (
            <NativeKeyRotation
              channelId={channelId}
              endpoint={rotationChannel.endpoint}
              protocol={rotationChannel.protocol}
              activeReference={keys.active_reference}
              disabled={!writable}
              onVerify={(candidate, signal) =>
                verifyReplacementCredential(scope, candidate, signal)
              }
              onSwitch={switchCredential}
            />
          ) : rotationChannel ? (
            <p role='status' className='text-muted-foreground text-sm'>
              {keys.polling
                ? t('Turn off key polling to replace the active key.')
                : t(
                    'Refresh channel capabilities to verify a replacement key.'
                  )}
            </p>
          ) : null}
          <div className='space-y-2'>
            {keys.keys.map((key) => {
              const active =
                key.reference.id === keys.active_reference.id &&
                key.reference.revision === keys.active_reference.revision
              const id = `key-${key.reference.id}-${key.reference.revision}`
              return (
                <div
                  key={id}
                  className='flex min-w-0 flex-wrap items-center justify-between gap-3 border-b py-4 last:border-b-0'
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
                        busy ||
                        !writable ||
                        (active && (key.enabled || !keys.polling))
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
                      disabled={busy || !writable || active}
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
          <section
            className='space-y-4 border-t pt-5'
            aria-label={t('Add API keys')}
          >
            <div
              className='flex flex-wrap gap-2'
              role='group'
              aria-label={t('Add API keys')}
            >
              <Button
                type='button'
                variant={batch ? 'ghost' : 'secondary'}
                className='min-h-11'
                aria-pressed={!batch}
                disabled={busy}
                onClick={() => {
                  setBatch(false)
                  setBatchSecrets('')
                  setImported(null)
                }}
              >
                {t('Single key')}
              </Button>
              {bulkImportAllowed && (
                <Button
                  type='button'
                  variant={batch ? 'secondary' : 'ghost'}
                  className='min-h-11'
                  aria-pressed={batch}
                  disabled={busy || !writable}
                  onClick={() => {
                    setBatch(true)
                    setSecret('')
                    setImported(null)
                  }}
                >
                  {t('Batch import')}
                </Button>
              )}
            </div>
            {imported !== null && (
              <p role='status' className='text-sm'>
                {t('Imported: {{number}}', { number: imported })}
              </p>
            )}
            <form
              className='grid min-w-0 gap-4 sm:grid-cols-2'
              onSubmit={(event) => {
                event.preventDefault()
                if (!writable || !label.trim()) return
                if (batch) {
                  if (!bulkImportAllowed) return
                  const values = batchSecrets
                    .split(/\r?\n/)
                    .map((value) => value.trim())
                    .filter(Boolean)
                  if (
                    values.length < 1 ||
                    values.length > 100 ||
                    new Set(values).size !== values.length
                  ) {
                    setError(t('Enter 1 to 100 different keys, one per line.'))
                    return
                  }
                  void change({
                    operation: 'add_many',
                    keys: values.map((value, index) => ({
                      label: `${label.trim()} ${index + 1}`,
                      secret: value,
                    })),
                    polling: keys.polling,
                  })
                } else {
                  if (!secret) return
                  void change({
                    operation: 'add',
                    label: label.trim(),
                    secret,
                    activate: false,
                  })
                }
              }}
            >
              <label className='block min-w-0 space-y-2'>
                <span className='text-sm'>
                  {batch ? t('Name prefix') : t('Name')}
                </span>
                <Input
                  className='dark:bg-background min-h-11'
                  value={label}
                  maxLength={batch ? 124 : 128}
                  required
                  disabled={busy}
                  onChange={(event) => setLabel(event.target.value)}
                />
              </label>
              {batch ? (
                <label className='block min-w-0 space-y-2 sm:col-span-2'>
                  <span className='text-sm'>{t('API Keys')}</span>
                  <Textarea
                    className='dark:bg-background min-h-40 font-mono text-sm'
                    autoComplete='off'
                    spellCheck={false}
                    value={batchSecrets}
                    required
                    disabled={busy}
                    onChange={(event) => setBatchSecrets(event.target.value)}
                  />
                  <span className='text-muted-foreground block text-sm'>
                    {t('Enter 1 to 100 different keys, one per line.')}
                  </span>
                </label>
              ) : (
                <label className='block min-w-0 space-y-2'>
                  <span className='text-sm'>{t('API key')}</span>
                  <Input
                    className='dark:bg-background min-h-11'
                    type='password'
                    autoComplete='off'
                    value={secret}
                    required
                    disabled={busy}
                    onChange={(event) => setSecret(event.target.value)}
                  />
                </label>
              )}
              <div className='flex flex-wrap gap-2 sm:col-span-2'>
                <Button
                  type='submit'
                  className='min-h-11'
                  disabled={busy || !writable}
                >
                  {batch ? t('Import keys') : t('Add')}
                </Button>
                <Button
                  type='button'
                  variant='ghost'
                  className='min-h-11'
                  disabled={busy}
                  onClick={() => {
                    setSecret('')
                    setBatchSecrets('')
                    setLabel('')
                    setImported(null)
                  }}
                >
                  {t('Clear')}
                </Button>
              </div>
            </form>
          </section>
        </>
      )}
    </Dialog>
  )
}
