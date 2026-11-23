/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { Mail } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { useAuthStore } from '@/stores/auth-store'

import type { InvitationAction } from './assistant-invitation-contract'
import {
  createInvitationRuntime,
  type InvitationOwner,
  type InvitationState,
} from './assistant-invitation-runtime'

function currentOwner(): InvitationOwner | undefined {
  const { user, session, accessToken } = useAuthStore.getState().auth
  if (!user || !session?.sid || !accessToken) return
  return { userID: user.id, sessionID: session.sid, accessToken }
}

export function AssistantInvitationCard({
  action,
  disabled = false,
}: {
  action: InvitationAction
  disabled?: boolean
}) {
  const { t } = useTranslation()
  const runtime = useRef<ReturnType<typeof createInvitationRuntime> | null>(
    null
  )
  const [snapshot, setSnapshot] = useState<{
    action: InvitationAction
    state: InvitationState
    canOpen: boolean
    canSend: boolean
    active: boolean
  } | null>(null)
  const current = snapshot?.action === action ? snapshot : null
  const state: InvitationState = disabled
    ? { status: 'unavailable' }
    : (current?.state ?? { status: 'loading' })
  useEffect(() => {
    if (disabled) return
    const owner = currentOwner()
    const controller = createInvitationRuntime(action, {
      owner: currentOwner,
      async request(method, path, signal) {
        if (!owner?.accessToken) {
          throw new Error('Invitation session unavailable')
        }
        const response = await fetch(path, {
          method,
          signal,
          credentials: 'include',
          cache: 'no-store',
          headers: {
            'Content-Type': 'application/json',
            Authorization: `Bearer ${owner.accessToken}`,
          },
          ...(method === 'POST' ? { body: '{}' } : {}),
        })
        if (!response.ok) throw new Error('Invitation request unavailable')
        return response.json()
      },
    })
    runtime.current = controller
    const unsubscribe = controller.subscribe(() => {
      setSnapshot({
        action,
        state: controller.state(),
        canOpen: controller.canOpen(),
        canSend: controller.canSend(),
        active: controller.active(),
      })
    })
    const unsubscribeAuth = useAuthStore.subscribe(() => {
      controller.checkOwner()
    })
    // Read-only refresh; StrictMode/remount never submits a confirmation or mail.
    void controller.refresh()
    const timer = setTimeout(
      () => controller.expire(),
      Math.min(300_001, Math.max(0, action.expires_at * 1000 - Date.now() + 1))
    )
    return () => {
      clearTimeout(timer)
      unsubscribe()
      unsubscribeAuth()
      controller.invalidate()
      if (runtime.current === controller) runtime.current = null
    }
  }, [action, disabled])

  const labels = {
    loading: t('Reading the invitation confirmation status.'),
    awaiting_confirmation: t(
      'Review the recipient and invitation code in the confirmation page. Then refresh this status.'
    ),
    ready: t(
      'Your browser confirmation is verified. You can now send this invitation once.'
    ),
    sending: t('Submitting this invitation once. Do not start another send.'),
    accepted_by_smtp: t(
      'The mail server accepted the invitation. Delivery is not confirmed.'
    ),
    rejected: t(
      'The mail server rejected this invitation. This operation will not resend it.'
    ),
    not_dispatched: t(
      'No email was sent by this operation. It will not automatically retry.'
    ),
    unknown: t(
      'The send outcome is unknown. Check status only; this operation will not send again.'
    ),
    expired: t(
      'This invitation confirmation expired. You can still check its send status.'
    ),
    unavailable: t('This invitation is unavailable for the current session.'),
  }
  const busy = state.status === 'loading' || state.status === 'sending'
  const showDetails =
    !disabled && state.status !== 'unavailable' && !!state.view
  return (
    <section
      className='bg-muted/30 w-full min-w-0 space-y-3 rounded-2xl px-5 py-4'
      aria-label={t('Invitation email')}
    >
      <h3 className='flex items-center gap-2 text-sm font-medium'>
        <Mail className='size-4' aria-hidden='true' />
        {t('Invitation email')}
      </h3>
      {showDetails && (
        <dl className='space-y-2 text-sm'>
          <div>
            <dt className='text-muted-foreground text-xs'>
              {t('Email address')}
            </dt>
            <dd className='break-all'>{state.view?.payload.email}</dd>
          </div>
          <div>
            <dt className='text-muted-foreground text-xs'>
              {t('Invitation code')}
            </dt>
            <dd className='break-all'>{state.view?.payload.invitation_code}</dd>
          </div>
        </dl>
      )}
      <p
        role={state.status === 'unavailable' ? 'alert' : 'status'}
        className='text-sm'
      >
        {labels[state.status]}
      </p>
      <div className='flex flex-wrap items-center gap-3'>
        {!disabled && current?.canOpen && state.view ? (
          <a
            href={state.view.browser_url}
            target='_blank'
            rel='noopener noreferrer'
            className='text-primary focus-visible:ring-ring/50 inline-flex min-h-11 items-center rounded-md px-1 text-sm underline underline-offset-4 outline-none focus-visible:ring-2'
            onClick={(event) => {
              if (!runtime.current?.canOpen()) event.preventDefault()
            }}
          >
            {t('Review invitation')}
          </a>
        ) : null}
        <Button
          type='button'
          className='h-auto min-h-11 max-w-full whitespace-normal'
          variant='outline'
          disabled={disabled || busy || !current?.active}
          onClick={() => {
            void runtime.current?.refresh()
          }}
        >
          {t('Check invitation status')}
        </Button>
        <Button
          type='button'
          className='h-auto min-h-11 max-w-full whitespace-normal'
          disabled={disabled || !current?.canSend}
          onClick={() => {
            void runtime.current?.send()
          }}
        >
          {t('Send invitation once')}
        </Button>
      </div>
      <p className='text-muted-foreground text-xs'>
        {t(
          'After checking the result, resume the original assistant turn. No new invitation is created by resuming.'
        )}
      </p>
    </section>
  )
}
