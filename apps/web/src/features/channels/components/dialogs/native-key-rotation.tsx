/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { isAxiosError } from 'axios'
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'

import type { NativeCredentialRef } from '../../native-api'

type RotationProps = {
  channelId: number
  endpoint: string
  protocol: string
  activeReference: NativeCredentialRef
  disabled: boolean
  onVerify: (
    candidate: { endpoint: string; protocol: string; secret: string },
    signal: AbortSignal
  ) => Promise<void>
  onSwitch?: (candidate: { label: string; secret: string }) => Promise<void>
}

export function NativeKeyRotation(props: RotationProps) {
  const { channelId, endpoint, protocol, activeReference, disabled } = props
  // A verified key belongs to this exact destination and active credential.
  const key = JSON.stringify([
    channelId,
    endpoint,
    protocol,
    activeReference.id,
    activeReference.revision,
    Boolean(props.onSwitch),
    disabled,
  ])
  return <RotationForm key={key} {...props} />
}

function RotationForm({
  endpoint,
  protocol,
  disabled,
  onVerify,
  onSwitch,
}: RotationProps) {
  const { t } = useTranslation()
  const [label, setLabel] = useState('')
  const [secret, setSecret] = useState('')
  const [state, setState] = useState<
    | 'draft'
    | 'verifying'
    | 'verified'
    | 'verifyFailed'
    | 'switching'
    | 'switched'
    | 'switchFailed'
  >('draft')
  const [probeHint, setProbeHint] = useState('')
  const proof = useRef<AbortController | null>(null)
  const switching = useRef(false)
  useEffect(() => () => proof.current?.abort(), [])

  function invalidate() {
    proof.current?.abort()
    proof.current = null
    setState('draft')
  }

  async function verify() {
    if (
      disabled ||
      (onSwitch !== undefined && !label.trim()) ||
      !secret.trim() ||
      proof.current ||
      switching.current
    ) {
      return
    }
    const attempt = new AbortController()
    proof.current = attempt
    setState('verifying')
    try {
      await onVerify({ endpoint, protocol, secret }, attempt.signal)
      if (!attempt.signal.aborted) {
        if (!onSwitch) setSecret('')
        setState('verified')
      }
    } catch (error) {
      if (attempt.signal.aborted) return
      const code = isAxiosError(error) ? error.response?.data?.code : null
      switch (code) {
        case 'CHANNEL_CATALOG_UPSTREAM_UNAUTHENTICATED':
          setProbeHint(t('Unauthorized'))
          break
        case 'CHANNEL_CATALOG_UPSTREAM_FORBIDDEN':
          setProbeHint(t('Access denied'))
          break
        case 'CHANNEL_CATALOG_UPSTREAM_RATE_LIMIT':
          setProbeHint(t('Too many requests'))
          break
        case 'CHANNEL_CATALOG_CONNECTION_FAILED':
          setProbeHint(t('Connection failed'))
          break
        default:
          setProbeHint('')
      }
      proof.current = null
      setState('verifyFailed')
    }
  }

  async function switchKey() {
    if (
      disabled ||
      !onSwitch ||
      state !== 'verified' ||
      !proof.current ||
      switching.current
    ) {
      return
    }
    switching.current = true
    proof.current = null
    setState('switching')
    const replacement = { label: label.trim(), secret }
    setSecret('')
    try {
      await onSwitch(replacement)
      setLabel('')
      setState('switched')
    } catch {
      setState('switchFailed')
    } finally {
      switching.current = false
    }
  }

  const fieldsDisabled = disabled || state === 'switching'
  return (
    <section
      className='min-w-0 space-y-3'
      aria-label={
        onSwitch ? t('Replace active key') : t('Verify replacement key')
      }
    >
      <h4 className='text-sm font-medium'>
        {onSwitch ? t('Replace active key') : t('Verify replacement key')}
      </h4>
      <div className='grid min-w-0 gap-4 sm:grid-cols-2'>
        {onSwitch && (
          <label className='block min-w-0 space-y-2 text-sm'>
            <span>{t('Name')}</span>
            <Input
              className='min-h-11'
              maxLength={128}
              value={label}
              disabled={fieldsDisabled}
              onChange={(event) => {
                invalidate()
                setLabel(event.target.value)
              }}
            />
          </label>
        )}
        <label className='block min-w-0 space-y-2 text-sm'>
          <span>{t('API key')}</span>
          <Input
            className='min-h-11'
            type='password'
            autoComplete='off'
            spellCheck={false}
            value={secret}
            disabled={fieldsDisabled}
            onChange={(event) => {
              invalidate()
              setSecret(event.target.value)
            }}
          />
        </label>
      </div>
      <div className='flex flex-wrap gap-2'>
        <Button
          type='button'
          variant='outline'
          className='min-h-11'
          disabled={
            fieldsDisabled ||
            (onSwitch !== undefined && !label.trim()) ||
            !secret.trim() ||
            state === 'verifying' ||
            state === 'verified'
          }
          onClick={() => void verify()}
        >
          {state === 'verifying' ? t('Loading') : t('Verify replacement key')}
        </Button>
        {onSwitch && (
          <Button
            type='button'
            className='min-h-11'
            disabled={fieldsDisabled || state !== 'verified'}
            onClick={() => void switchKey()}
          >
            {t('Confirm key switch')}
          </Button>
        )}
      </div>
      {state === 'verified' && (
        <p role='status' className='text-sm'>
          {onSwitch
            ? t(
                'The replacement key was verified. Confirm to use it for new requests.'
              )
            : t('Connection successful')}
        </p>
      )}
      {state === 'switched' && (
        <p role='status' className='text-sm'>
          {t('Success')}
        </p>
      )}
      {state === 'verifyFailed' && (
        <p role='alert' className='text-destructive text-sm'>
          {t('The replacement key could not be verified.')} {probeHint}
        </p>
      )}
      {state === 'switchFailed' && (
        <p role='alert' className='text-destructive text-sm'>
          {t('Check the active key before trying another switch.')}
        </p>
      )}
    </section>
  )
}
