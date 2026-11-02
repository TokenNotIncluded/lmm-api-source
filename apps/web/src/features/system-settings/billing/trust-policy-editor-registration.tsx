/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'

import {
  createPublicRegistration,
  parsePublicRegistration,
  REGISTRATION_MAX_BYTES,
  type PublicRegistration,
} from './trust-policy-editor-registration-contract'

/** Public, offline transport only. This component has no API or owner connection. */
export function TrustPolicyEditorRegistration() {
  const { t } = useTranslation()
  const [raw, setRaw] = useState('')
  const [input, setInput] = useState<PublicRegistration | null>(null)
  const [accepted, setAccepted] = useState(false)
  const [output, setOutput] = useState<string | null>(null)
  const [error, setError] = useState(false)
  const [busy, setBusy] = useState(false)
  const operation = useRef<AbortController | null>(null)
  const generation = useRef(0)
  useEffect(
    () => () => {
      generation.current++
      operation.current?.abort()
    },
    []
  )
  const reset = (value = '') => {
    generation.current++
    operation.current?.abort()
    operation.current = null
    setRaw(value)
    setInput(null)
    setAccepted(false)
    setOutput(null)
    setError(false)
    setBusy(false)
  }
  const load = async (file?: File) => {
    reset()
    if (!file) return
    const current = generation.current
    try {
      if (file.size > REGISTRATION_MAX_BYTES) throw new Error('Too large')
      const text = await file.text()
      if (generation.current === current) setRaw(text)
    } catch {
      if (generation.current === current) setError(true)
    }
  }
  const inspect = () => {
    setError(false)
    setOutput(null)
    setAccepted(false)
    setInput(null)
    try {
      setInput(parsePublicRegistration(raw, window.location.origin))
    } catch {
      setError(true)
    }
  }
  const create = async () => {
    if (!input || !accepted || operation.current || output) return
    const controller = new AbortController()
    operation.current = controller
    const current = generation.current
    setBusy(true)
    setError(false)
    try {
      const response = await createPublicRegistration(input, controller.signal)
      if (current === generation.current) setOutput(response)
    } catch {
      if (current === generation.current) setError(true)
    } finally {
      if (current === generation.current) {
        setBusy(false)
        operation.current = null
        setAccepted(false)
      }
    }
  }
  const download = () => {
    if (!output) return
    const url = URL.createObjectURL(
      new Blob([output], { type: 'application/json' })
    )
    const a = document.createElement('a')
    a.href = url
    a.download = 'trust-policy-credential-response.json'
    a.click()
    setTimeout(() => URL.revokeObjectURL(url), 1000)
  }
  return (
    <section
      className='space-y-4'
      aria-label={t('Offline editor credential creation')}
    >
      <h3 className='text-base font-semibold'>
        {t('Offline editor credential creation')}
      </h3>
      <Alert role='note'>
        <AlertDescription>
          {t(
            'This only creates a browser credential. The owner must review and approve it offline before it can edit policy.'
          )}
        </AlertDescription>
      </Alert>
      <label className='block text-sm font-medium' htmlFor='registration-file'>
        {t('Import public CLI begin JSON')}
      </label>
      <Input
        id='registration-file'
        type='file'
        accept='.json,application/json'
        disabled={busy}
        onChange={(event) => {
          void load(event.target.files?.[0])
          event.target.value = ''
        }}
      />
      <label
        className='block text-sm font-medium'
        htmlFor='registration-options'
      >
        {t('Public registration options JSON')}
      </label>
      <Textarea
        id='registration-options'
        value={raw}
        disabled={busy}
        rows={7}
        maxLength={REGISTRATION_MAX_BYTES}
        autoComplete='off'
        spellCheck={false}
        onChange={(event) => reset(event.target.value)}
      />
      <div className='flex flex-wrap gap-2'>
        <Button
          type='button'
          variant='outline'
          disabled={busy || !raw}
          onClick={inspect}
        >
          {t('Inspect public options')}
        </Button>
        <Button type='button' variant='outline' onClick={() => reset()}>
          {t('Clear registration data')}
        </Button>
      </div>
      {error && (
        <Alert variant='destructive' role='alert'>
          <AlertDescription>
            {t(
              'Creation did not complete. Check the public options and browser origin, or ask the owner for fresh options. No approval was performed.'
            )}
          </AlertDescription>
        </Alert>
      )}
      {input && (
        <div className='space-y-3'>
          <dl className='grid gap-2 text-sm break-all'>
            <dt>{t('Editor label from public options')}</dt>
            <dd>{input.options.publicKey.user.name}</dd>
            <dt>{t('Relying party ID')}</dt>
            <dd>{input.options.publicKey.rp.id}</dd>
            <dt>{t('This browser origin')}</dt>
            <dd>{window.location.origin}</dd>
            <dt>{t('Enrollment reference')}</dt>
            <dd>{input.enrollment}</dd>
          </dl>
          <p className='text-muted-foreground text-sm'>
            {t(
              'These options do not reveal the actor ID or configured origin. Verify the exact origin, including port, with the owner. The CLI confirmed begin, not approval.'
            )}
          </p>
          {!output && (
            <>
              <div className='flex items-center gap-2'>
                <Checkbox
                  id='registration-reviewed'
                  checked={accepted}
                  disabled={busy}
                  onCheckedChange={(v) => setAccepted(v === true)}
                />
                <label htmlFor='registration-reviewed' className='text-sm'>
                  {t(
                    'I checked the editor label and exact origin with the owner.'
                  )}
                </label>
              </div>
              <Button
                type='button'
                disabled={busy || !accepted}
                onClick={() => void create()}
              >
                {t('Create credential only')}
              </Button>
            </>
          )}
        </div>
      )}
      {output && (
        <div className='space-y-3'>
          <Alert role='status'>
            <AlertDescription>
              {t(
                'Creation only — awaiting owner offline review and approval. This page does not confirm editor enrollment.'
              )}
            </AlertDescription>
          </Alert>
          <label
            className='block text-sm font-medium'
            htmlFor='registration-response'
          >
            {t('Public credential response JSON')}
          </label>
          <Textarea
            id='registration-response'
            readOnly
            value={output}
            rows={8}
            spellCheck={false}
          />
          <Button type='button' onClick={download}>
            {t('Download public credential response')}
          </Button>
        </div>
      )}
    </section>
  )
}
