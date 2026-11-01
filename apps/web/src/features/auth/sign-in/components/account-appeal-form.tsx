/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import axios from 'axios'
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { Turnstile } from '@/components/turnstile'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Textarea } from '@/components/ui/textarea'
import { useTurnstile } from '@/features/auth/hooks/use-turnstile'

import { getAccountAppealContext, submitAccountAppeal } from '../../api'
import {
  appealStatusKeys,
  type AccountAppealContext,
  type AccountAppealReceipt,
} from '../../lib/account-appeal'

export function AccountAppealForm({ refreshKey = 0 }: { refreshKey?: number }) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const [context, setContext] = useState<AccountAppealContext | null>(null)
  const [receipt, setReceipt] = useState<AccountAppealReceipt | null>(null)
  const [loading, setLoading] = useState(true)
  const [expired, setExpired] = useState(false)
  const [username, setUsername] = useState('')
  const [password, setPassword] = useState('')
  const [reason, setReason] = useState('')
  const [submitting, setSubmitting] = useState(false)
  const pending = useRef(false)
  const identityRequest = useRef({ active: false })
  const {
    isTurnstileEnabled,
    turnstileSiteKey,
    turnstileToken,
    setTurnstileToken,
    validateTurnstile,
  } = useTurnstile()

  useEffect(() => {
    let active = true
    const identity = { active: true }
    identityRequest.current = identity
    setReason('')
    setPassword('')
    setLoading(true)
    setContext(null)
    setReceipt(null)
    void getAccountAppealContext()
      .then((value) => {
        if (!active) return
        setContext(value)
        setExpired(value.expires_at <= Date.now() / 1000)
        setOpen(true)
      })
      .catch((error: unknown) => {
        if (!active) return
        if (axios.isAxiosError(error) && error.response?.status === 401) {
          setExpired(refreshKey > 0)
        } else {
          toast.error(t('Unable to load account appeal'))
        }
      })
      .finally(() => {
        if (active) setLoading(false)
      })
    return () => {
      active = false
      identity.active = false
    }
  }, [refreshKey, t])

  useEffect(() => {
    if (!context) return
    const timeout = setTimeout(
      () => setExpired(true),
      Math.max(0, context.expires_at * 1000 - Date.now())
    )
    return () => clearTimeout(timeout)
  }, [context])

  const request = receipt ?? context?.request
  const locked = request?.status === 'pending' || request?.status === 'approved'
  const submit = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (
      pending.current ||
      loading ||
      locked ||
      expired ||
      reason.trim().length < 5
    ) {
      return
    }
    if (context && context.expires_at <= Date.now() / 1000) {
      setExpired(true)
      return
    }
    if (!validateTurnstile()) return
    pending.current = true
    const identity = identityRequest.current
    setSubmitting(true)
    try {
      const response = await submitAccountAppeal(
        context
          ? { reason: reason.trim(), turnstile: turnstileToken }
          : {
              username,
              password,
              reason: reason.trim(),
              turnstile: turnstileToken,
            }
      )
      if (!identity.active) return
      if (response.data) {
        setReceipt(response.data)
        setPassword('')
      }
    } catch (error: unknown) {
      if (!identity.active) return
      if (
        axios.isAxiosError(error) &&
        error.response?.status === 401 &&
        context
      ) {
        setExpired(true)
      } else {
        toast.error(t('Unable to submit appeal'))
      }
    } finally {
      pending.current = false
      setSubmitting(false)
    }
  }

  if (!open) {
    return (
      <Button
        type='button'
        variant='link'
        className='h-auto px-0 text-sm'
        onClick={() => setOpen(true)}
      >
        {t('Account disabled? Submit an appeal')}
      </Button>
    )
  }

  return (
    <form
      className='grid gap-3 rounded-lg border p-4'
      onSubmit={submit}
      aria-label={t('Request account restoration')}
    >
      <p className='text-sm font-medium'>{t('Request account restoration')}</p>
      {context ? (
        <div className='grid gap-2 text-sm'>
          <p>
            {t('Verified account')}: {context.username}
          </p>
          <p className='whitespace-pre-wrap'>
            {context.public_reason_available && context.public_reason !== null
              ? context.public_reason
              : t('No public reason has been provided.')}
          </p>
          <p className='text-muted-foreground'>
            {t(
              'Submit an appeal using your verified identity. No local password is needed.'
            )}
          </p>
        </div>
      ) : (
        <>
          <p className='text-muted-foreground text-xs'>
            {t(
              'Verify the disabled account with its username or email and password. An administrator must approve the appeal.'
            )}
          </p>
          <Label htmlFor='appeal-username'>{t('Username or Email')}</Label>
          <Input
            id='appeal-username'
            value={username}
            onChange={(event) => setUsername(event.target.value)}
            autoComplete='username'
            required
          />
          <Label htmlFor='appeal-password'>{t('Password')}</Label>
          <Input
            id='appeal-password'
            type='password'
            value={password}
            onChange={(event) => setPassword(event.target.value)}
            autoComplete='current-password'
            required
          />
        </>
      )}
      {loading ? <p role='status'>{t('Loading...')}</p> : null}
      {request ? (
        <div role='status' className='grid gap-2 text-sm'>
          <p>{t(appealStatusKeys[request.status])}</p>
          <p className='whitespace-pre-wrap'>{request.reason}</p>
          {request.status === 'approved' ? (
            <p>{t('Your appeal was approved. Sign in again to continue.')}</p>
          ) : null}
        </div>
      ) : null}
      {expired ? (
        <p role='alert'>
          {t(
            'Appeal verification expired. Sign in again to verify your identity.'
          )}
        </p>
      ) : null}
      {!locked ? (
        <>
          <Label htmlFor='appeal-reason'>{t('Request reason')}</Label>
          <Textarea
            id='appeal-reason'
            value={reason}
            onChange={(event) => setReason(event.target.value)}
            placeholder={t('Explain why this account should be restored.')}
            minLength={5}
            maxLength={2000}
            required
          />
          {isTurnstileEnabled ? (
            <Turnstile
              siteKey={turnstileSiteKey}
              onVerify={setTurnstileToken}
            />
          ) : null}
          <Button
            type='submit'
            disabled={
              loading ||
              submitting ||
              expired ||
              (!context && (!username.trim() || !password)) ||
              reason.trim().length < 5
            }
          >
            {t('Send appeal')}
          </Button>
        </>
      ) : null}
      <Button type='button' variant='outline' onClick={() => setOpen(false)}>
        {t('Cancel')}
      </Button>
    </form>
  )
}
