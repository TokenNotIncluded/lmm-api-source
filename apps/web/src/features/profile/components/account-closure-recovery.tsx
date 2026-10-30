/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { clearAuthenticationIfCurrent } from '@/lib/auth-session'
import { useAuthStore } from '@/stores/auth-store'

import {
  clearPendingClosure,
  readClosureReceipt,
  readPendingClosure,
} from '../account-lifecycle-api'

export function AccountClosureRecovery() {
  const { t } = useTranslation()
  const [pending, setPending] = useState(() => {
    try {
      return typeof window === 'undefined' ? null : readPendingClosure()
    } catch {
      return null
    }
  })
  const [loading, setLoading] = useState(false)
  const [confirmed, setConfirmed] = useState(false)
  if (!pending) {
    return confirmed ? (
      <Alert>
        <AlertDescription>{t('Account closed successfully')}</AlertDescription>
      </Alert>
    ) : null
  }
  const check = async () => {
    setLoading(true)
    try {
      await readClosureReceipt(pending)
      const auth = useAuthStore.getState().auth
      if (auth.user?.id === pending.user_id) {
        clearAuthenticationIfCurrent({
          userId: auth.user.id,
          sessionId: auth.session?.sid,
        })
      }
      try {
        clearPendingClosure(pending.operation_key, pending.user_id)
      } catch {
        /* Preserve the confirmed result even when local cleanup is unavailable. */
      }
      setPending(null)
      setConfirmed(true)
    } finally {
      setLoading(false)
    }
  }
  return (
    <Alert>
      <AlertDescription className='space-y-2'>
        <p>
          {t(
            'Account closure is not confirmed. Check the receipt before trying again.'
          )}
        </p>
        <Button
          type='button'
          size='sm'
          variant='outline'
          disabled={loading}
          onClick={() => {
            void check().catch(() => {})
          }}
        >
          {t('Check closure receipt')}
        </Button>
      </AlertDescription>
    </Alert>
  )
}
