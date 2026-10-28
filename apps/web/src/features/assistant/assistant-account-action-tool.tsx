/*
Copyright (C) 2026 LIghtJUNction
*/
import { AlertTriangle, Check, ShieldAlert } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { Spinner } from '@/components/ui/spinner'

import {
  submitAssistantAccountDisableRequest,
  type AssistantAccountDisableAction,
} from './api'

export function AssistantAccountActionTool(props: {
  action: AssistantAccountDisableAction
  onSubmitted?: () => void
}) {
  const { t } = useTranslation()
  const [submitting, setSubmitting] = useState(false)
  const [submitted, setSubmitted] = useState(false)

  const submit = async () => {
    if (submitting || submitted) return
    setSubmitting(true)
    try {
      await submitAssistantAccountDisableRequest({
        target_user_id: props.action.target_user_id,
        reason: props.action.reason,
        confirmation_token: props.action.confirmation_token,
      })
      setSubmitted(true)
      props.onSubmitted?.()
      toast.success(t('Account safety request sent to an administrator'))
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : t('Unable to submit the account safety request')
      )
    } finally {
      setSubmitting(false)
    }
  }

  if (submitted) {
    return (
      <Card size='sm' className='border-primary/30 bg-primary/5'>
        <CardHeader>
          <CardTitle className='flex items-center gap-2'>
            <Check className='size-4' aria-hidden='true' />
            {t('Account safety request submitted')}
          </CardTitle>
          <CardDescription>
            {t(
              'The administrator will review this request. The account has not been disabled automatically.'
            )}
          </CardDescription>
        </CardHeader>
      </Card>
    )
  }

  return (
    <Card size='sm' className='border-destructive/30'>
      <CardHeader>
        <CardTitle className='flex items-center gap-2'>
          <ShieldAlert className='text-destructive size-4' aria-hidden='true' />
          {t('Review account safety request')}
        </CardTitle>
        <CardDescription>
          {t(
            'This sends a recommendation to an administrator. It does not disable an account until an administrator approves it.'
          )}
        </CardDescription>
      </CardHeader>
      <CardContent className='grid gap-3'>
        <Alert variant='destructive'>
          <AlertTriangle className='size-4' aria-hidden='true' />
          <AlertTitle>
            {t('Target account')}: {props.action.target_username}
          </AlertTitle>
          <AlertDescription className='whitespace-pre-wrap'>
            {props.action.reason}
          </AlertDescription>
        </Alert>
        <Button
          type='button'
          variant='destructive'
          onClick={() => void submit()}
          disabled={submitting}
        >
          {submitting ? <Spinner data-icon='inline-start' /> : null}
          {t('Confirm and send for administrator review')}
        </Button>
      </CardContent>
    </Card>
  )
}
