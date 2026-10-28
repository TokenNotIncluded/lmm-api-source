/*
Copyright (C) 2026 LIghtJUNction
*/
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { Turnstile } from '@/components/turnstile'
import { Button } from '@/components/ui/button'
import { Textarea } from '@/components/ui/textarea'
import { useTurnstile } from '@/features/auth/hooks/use-turnstile'

import { submitAccountAppeal } from '../../api'

const minReasonLength = 5

export function AccountAppealForm() {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const [username, setUsername] = useState('')
  const [password, setPassword] = useState('')
  const [reason, setReason] = useState('')
  const [submitting, setSubmitting] = useState(false)
  const [submitted, setSubmitted] = useState(false)
  const {
    isTurnstileEnabled,
    turnstileSiteKey,
    turnstileToken,
    setTurnstileToken,
    validateTurnstile,
  } = useTurnstile()

  const submit = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (submitting || reason.trim().length < minReasonLength) return
    if (!validateTurnstile()) return
    setSubmitting(true)
    try {
      const response = await submitAccountAppeal({
        username,
        password,
        reason: reason.trim(),
        turnstile: turnstileToken,
      })
      if (!response.success) {
        throw new Error(response.message || t('Unable to submit appeal'))
      }
      setSubmitted(true)
      toast.success(t('Appeal sent to an administrator'))
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : t('Unable to submit appeal')
      )
    } finally {
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

  if (submitted) {
    return (
      <p className='text-muted-foreground text-sm leading-6'>
        {t(
          'Your appeal is pending administrator review. You will be able to sign in after it is approved.'
        )}
      </p>
    )
  }

  return (
    <form className='grid gap-3 rounded-lg border p-4' onSubmit={submit}>
      <div>
        <p className='text-sm font-medium'>
          {t('Request account restoration')}
        </p>
        <p className='text-muted-foreground mt-1 text-xs leading-5'>
          {t(
            'Verify the disabled account with its username or email and password. An administrator must approve the appeal.'
          )}
        </p>
      </div>
      <input
        className='border-input bg-background h-10 rounded-lg border px-3 text-sm'
        value={username}
        onChange={(event) => setUsername(event.target.value)}
        placeholder={t('Username or Email')}
        autoComplete='username'
        required
      />
      <input
        className='border-input bg-background h-10 rounded-lg border px-3 text-sm'
        type='password'
        value={password}
        onChange={(event) => setPassword(event.target.value)}
        placeholder={t('Password')}
        autoComplete='current-password'
        required
      />
      <Textarea
        value={reason}
        onChange={(event) => setReason(event.target.value)}
        placeholder={t('Explain why this account should be restored.')}
        minLength={minReasonLength}
        maxLength={2000}
        required
      />
      {isTurnstileEnabled ? (
        <Turnstile siteKey={turnstileSiteKey} onVerify={setTurnstileToken} />
      ) : null}
      <div className='flex gap-2'>
        <Button type='button' variant='outline' onClick={() => setOpen(false)}>
          {t('Cancel')}
        </Button>
        <Button
          type='submit'
          disabled={
            submitting ||
            !username.trim() ||
            !password ||
            reason.trim().length < minReasonLength
          }
        >
          {t('Send appeal')}
        </Button>
      </div>
    </form>
  )
}
