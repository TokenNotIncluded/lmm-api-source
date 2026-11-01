/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { isAxiosError } from 'axios'
import { useEffect, useId, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { ZodError } from 'zod'

import { Dialog } from '@/components/dialog'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'

import {
  saveNativeBalanceProbe,
  type NativeBalanceProbe,
  type NativeRoutingScope,
  type NativeVersion,
} from '../../native-api'

export function NativeBalanceConfigDialog({
  scope,
  channelId,
  channelName,
  version,
  initialProbe,
  onSaved,
  onClose,
}: {
  scope: NativeRoutingScope
  channelId: number
  channelName: string
  version: NativeVersion
  initialProbe: NativeBalanceProbe | null
  onSaved: () => void
  onClose: () => void
}) {
  const { t } = useTranslation()
  const formId = useId()
  const [path, setPath] = useState(initialProbe?.path ?? '')
  const [pointer, setPointer] = useState(initialProbe?.json_pointer ?? '')
  const [unit, setUnit] = useState(initialProbe?.unit_label ?? '')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const pending = useRef(false)
  const active = useRef(true)
  useEffect(() => {
    active.current = true
    return () => {
      active.current = false
    }
  }, [])
  async function save(probe: NativeBalanceProbe | null) {
    if (pending.current) return
    pending.current = true
    setBusy(true)
    setError(null)
    try {
      await saveNativeBalanceProbe(scope, channelId, version, probe)
      if (!active.current) return
      onSaved()
      onClose()
    } catch (cause: unknown) {
      if (!active.current) return
      const status = isAxiosError(cause) ? cause.response?.status : undefined
      setError(
        t(
          cause instanceof ZodError
            ? 'channelBalance.invalid'
            : status === 403
              ? 'channelBalance.denied'
              : status === 409
                ? 'channelBalance.conflict'
                : 'channelBalance.saveError'
        )
      )
    } finally {
      pending.current = false
      if (active.current) setBusy(false)
    }
  }
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !pending.current) onClose()
      }}
      title={t('channelBalance.configure')}
      description={channelName}
      contentHeight='auto'
      footer={
        <>
          <Button
            variant='outline'
            disabled={busy}
            onClick={() => void save(null)}
          >
            {t('Disable')}
          </Button>
          <Button variant='outline' disabled={busy} onClick={onClose}>
            {t('Cancel')}
          </Button>
          <Button form={formId} type='submit' disabled={busy}>
            {t('Save')}
          </Button>
        </>
      }
    >
      <form
        id={formId}
        className='space-y-4'
        onSubmit={(event) => {
          event.preventDefault()
          void save({ path, json_pointer: pointer, unit_label: unit || null })
        }}
      >
        <p className='text-muted-foreground text-sm'>
          {t('channelBalance.configNote')}
        </p>
        <div className='space-y-2'>
          <Label htmlFor={`${formId}-path`}>{t('channelBalance.path')}</Label>
          <Input
            id={`${formId}-path`}
            value={path}
            onChange={(event) => setPath(event.target.value)}
            disabled={busy}
            placeholder='/balance'
            autoComplete='off'
          />
        </div>
        <div className='space-y-2'>
          <Label htmlFor={`${formId}-pointer`}>
            {t('channelBalance.pointer')}
          </Label>
          <Input
            id={`${formId}-pointer`}
            value={pointer}
            onChange={(event) => setPointer(event.target.value)}
            disabled={busy}
            placeholder='/data/balance'
            autoComplete='off'
          />
        </div>
        <div className='space-y-2'>
          <Label htmlFor={`${formId}-unit`}>{t('channelBalance.unit')}</Label>
          <Input
            id={`${formId}-unit`}
            value={unit}
            onChange={(event) => setUnit(event.target.value)}
            disabled={busy}
            autoComplete='off'
          />
        </div>
        {error && (
          <Alert variant='destructive' role='alert'>
            <AlertDescription>{error}</AlertDescription>
          </Alert>
        )}
      </form>
    </Dialog>
  )
}
