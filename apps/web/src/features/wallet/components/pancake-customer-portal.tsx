/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import {
  commerceScope,
  currentCommerceScope,
} from '@/features/commerce/native-client'

import {
  createPancakePortal,
  type PancakePortalLink,
} from '../pancake-portal-api'

export function PancakeCustomerPortal({ owner }: { owner: string }) {
  const { t, i18n } = useTranslation()
  const [link, setLink] = useState<PancakePortalLink | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<'unavailable' | 'expired' | null>(null)
  const mounted = useRef(true)
  const running = useRef(false)
  const current = () => {
    const scope = commerceScope()
    return mounted.current &&
      scope &&
      owner === JSON.stringify([scope.userId, scope.sessionId])
      ? scope
      : null
  }
  useEffect(() => {
    mounted.current = true
    return () => {
      mounted.current = false
    }
  }, [])
  useEffect(() => {
    if (!link) return
    const timer = setTimeout(
      () => {
        setLink(null)
        setError('expired')
      },
      Math.min(
        2_147_483_647,
        Math.max(0, Date.parse(link.expires_at) - Date.now())
      )
    )
    return () => clearTimeout(timer)
  }, [link])
  async function prepare() {
    const scope = current()
    if (!scope || running.current) return
    running.current = true
    setBusy(true)
    setError(null)
    setLink(null)
    try {
      const result = await createPancakePortal(scope)
      if (mounted.current && currentCommerceScope(scope)) setLink(result)
    } catch {
      if (mounted.current && currentCommerceScope(scope)) {
        setError('unavailable')
      }
    } finally {
      running.current = false
      if (mounted.current && currentCommerceScope(scope)) setBusy(false)
    }
  }
  return (
    <div className='space-y-3 border-t pt-4' aria-busy={busy}>
      <h3 className='text-sm font-medium'>{t('Pancake customer portal')}</h3>
      {link ? (
        <div className='flex flex-wrap items-center gap-3'>
          <Button
            variant='outline'
            render={
              <a
                href={link.portal_url}
                target='_blank'
                rel='noopener noreferrer'
                referrerPolicy='no-referrer'
                onClick={(event) => {
                  if (!current() || Date.parse(link.expires_at) <= Date.now()) {
                    event.preventDefault()
                    setLink(null)
                    setError('expired')
                  }
                }}
              />
            }
          >
            {t('Open payment portal')}
          </Button>
          <p className='text-muted-foreground text-xs'>
            {t('Expires at')}:{' '}
            <time dateTime={link.expires_at}>
              {new Date(link.expires_at).toLocaleString(
                i18n.resolvedLanguage || i18n.language
              )}
            </time>
          </p>
        </div>
      ) : (
        <Button
          variant='outline'
          disabled={busy}
          onClick={() => void prepare()}
        >
          {t('Create portal link')}
        </Button>
      )}
      {error && (
        <p role='alert' className='text-destructive text-sm'>
          {t(
            error === 'expired'
              ? 'Your portal link has expired. Create a new link to continue.'
              : 'Payment portal is unavailable. Try again later.'
          )}
        </p>
      )}
    </div>
  )
}
