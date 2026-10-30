/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { Link2, RefreshCw, Unlink } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { ConfirmDialog } from '@/components/confirm-dialog'
import { Button } from '@/components/ui/button'
import {
  OAUTH_BIND_CALLBACK_MESSAGE,
  OAUTH_BIND_RESULT_MESSAGE,
} from '@/features/auth/constants'
import { watchOAuthPopupClosed } from '@/features/auth/lib/oauth-bind-window'
import {
  getOAuthSessionStorage,
  markOAuthBindPopup,
} from '@/features/auth/lib/oauth-callback-mode'
import type { SystemStatus } from '@/features/auth/types'
import { useAuthStore } from '@/stores/auth-store'

import {
  accountLinkScope,
  assertAccountLinkScope,
  beginAccountBinding,
  finishAccountBinding,
  readAccountLinks,
  unlinkAccount,
  type AccountLink,
  type AccountLinkScope,
  type AccountLinks,
  type AccountProvider,
} from '../account-links-api'
import type { UserProfile } from '../types'

const labels: Record<AccountProvider, string> = {
  google: 'Google',
  github: 'GitHub',
  linuxdo: 'LinuxDO',
  discord: 'Discord',
}
const TELEGRAM = 'https://telegram.org'
type Pending = {
  provider: AccountProvider
  state: string
  popup: Window
  scope: AccountLinkScope
  stop: () => void
}

export function NativeAccountLinks({
  status,
  profile,
  onUpdate,
  onBindTelegram,
}: {
  status: SystemStatus
  profile: UserProfile
  onUpdate: () => void | Promise<void>
  onBindTelegram: () => void
}) {
  const { t } = useTranslation()
  const user = useAuthStore((state) => state.auth.user?.id)
  const sid = useAuthStore((state) => state.auth.session?.sid)
  const [view, setView] = useState<AccountLinks | null>(null)
  const [failed, setFailed] = useState(false)
  const [busy, setBusy] = useState(false)
  const [target, setTarget] = useState<AccountLink | null>(null)
  const pending = useRef<Pending | null>(null)
  const generation = useRef({ value: 0 })
  const clearPending = useCallback((expected?: Pending) => {
    const current = pending.current
    if (!current || (expected && current !== expected)) return
    current.stop()
    pending.current = null
    return current
  }, [])
  const load = useCallback(async () => {
    const current = ++generation.current.value
    setFailed(false)
    try {
      const result = await readAccountLinks(accountLinkScope())
      if (current === generation.current.value) setView(result)
    } catch {
      if (current === generation.current.value) {
        setView(null)
        setFailed(true)
      }
    }
  }, [])
  useEffect(() => {
    const lifecycle = generation.current
    setView(null)
    setTarget(null)
    setBusy(false)
    void load()
    return () => {
      lifecycle.value++
      clearPending()?.popup.close()
    }
  }, [user, sid, profile, load, clearPending])

  const bind = async (provider: AccountProvider) => {
    if (!view || busy) return
    const popup = window.open('', '_blank')
    if (!popup) {
      toast.error(t('OAuth pop-up was blocked'))
      return
    }
    setBusy(true)
    let flow: Pending | undefined
    try {
      const scope = accountLinkScope()
      flow = { provider, state: '', popup, scope, stop: () => undefined }
      pending.current = flow
      const started = await beginAccountBinding(
        scope,
        provider,
        view.expected_auth_version,
        status,
        window.location.origin
      )
      if (pending.current !== flow) return
      if (popup.closed) {
        clearPending(flow)
        setBusy(false)
        return
      }
      assertAccountLinkScope(scope)
      if (
        !markOAuthBindPopup(
          getOAuthSessionStorage(popup),
          provider,
          started.state
        )
      ) {
        throw new Error('Popup storage unavailable')
      }
      flow.state = started.state
      const captured = flow
      flow.stop = watchOAuthPopupClosed(popup, () => {
        clearPending(captured)
        setBusy(false)
      })
      // The API validates the fixed provider, client, callback and state before navigation.
      popup.location.replace(started.authorization_url)
    } catch {
      clearPending(flow)
      popup.close()
      setBusy(false)
      toast.error(
        t(
          'Account binding failed. Sign in again if your session is older than 10 minutes.'
        )
      )
    }
  }
  useEffect(() => {
    const callback = async (event: MessageEvent<unknown>) => {
      const current = pending.current
      const message = event.data as {
        type?: unknown
        provider?: unknown
        state?: unknown
        code?: unknown
        iss?: unknown
        error?: unknown
      } | null
      if (
        !current ||
        event.origin !== window.location.origin ||
        event.source !== current.popup ||
        !message ||
        message.type !== OAUTH_BIND_CALLBACK_MESSAGE ||
        message.provider !== current.provider ||
        message.state !== current.state
      ) {
        return
      }
      clearPending(current)
      let success = false
      try {
        if (
          message.error ||
          typeof message.code !== 'string' ||
          (message.iss !== undefined && typeof message.iss !== 'string')
        ) {
          throw new Error('Provider rejected binding')
        }
        await finishAccountBinding(current.scope, current.provider, {
          state: current.state,
          code: message.code,
          iss: message.iss as string | undefined,
        })
        success = true
        toast.success(t('Binding successful!'))
        await load()
        void onUpdate()
      } catch {
        toast.error(
          t(
            'Account binding could not be confirmed. Refresh the list before trying again.'
          )
        )
        void load()
      } finally {
        setBusy(false)
        if (!current.popup.closed) {
          current.popup.postMessage(
            {
              type: OAUTH_BIND_RESULT_MESSAGE,
              provider: current.provider,
              state: current.state,
              success,
              message: success ? undefined : t('Account binding failed'),
            },
            window.location.origin
          )
        }
      }
    }
    window.addEventListener('message', callback)
    return () => window.removeEventListener('message', callback)
  }, [clearPending, load, onUpdate, t])
  const unlink = async () => {
    if (!target || !view || busy) return
    const scope = accountLinkScope()
    setBusy(true)
    try {
      await unlinkAccount(scope, target, view.expected_auth_version)
      setTarget(null)
      toast.success(t('Unbound {{provider}}', { provider: target.issuer }))
      await load()
      void onUpdate()
    } catch {
      toast.error(
        t(
          'Account binding could not be confirmed. Refresh the list before trying again.'
        )
      )
      void load()
    } finally {
      setBusy(false)
    }
  }
  const rows: Array<{
    issuer: string
    label: string
    canBind: boolean
    onBind: () => void
  }> = view
    ? view.providers.map((provider) => ({
        issuer: provider.issuer,
        label: labels[provider.provider],
        canBind: provider.can_bind,
        onBind: () => void bind(provider.provider),
      }))
    : []
  if (
    view &&
    (status.telegram_oauth === true ||
      view.links.some((link) => link.issuer === TELEGRAM && link.active))
  ) {
    rows.push({
      issuer: TELEGRAM,
      label: 'Telegram',
      canBind: status.telegram_oauth === true,
      onBind: onBindTelegram,
    })
  }
  if (view) {
    for (const link of view.links) {
      if (link.active && !rows.some((row) => row.issuer === link.issuer)) {
        rows.push({
          issuer: link.issuer,
          label: link.issuer,
          canBind: false,
          onBind: () => undefined,
        })
      }
    }
  }
  return (
    <section
      className='mb-4 space-y-3'
      aria-label={t('Linked sign-in accounts')}
    >
      <div className='flex items-start justify-between gap-3'>
        <div>
          <h3 className='text-sm font-medium'>
            {t('Linked sign-in accounts')}
          </h3>
          <p className='text-muted-foreground text-xs'>
            {t(
              'Account security requires a sign-in within the last 10 minutes. Keep another sign-in method before unlinking.'
            )}
          </p>
        </div>
        <Button
          size='sm'
          variant='outline'
          disabled={busy}
          onClick={() => void load()}
          aria-label={t('Refresh')}
        >
          <RefreshCw className='size-4' />
        </Button>
      </div>
      {failed && (
        <p role='alert' className='text-destructive text-sm'>
          {t('Account bindings are unavailable')}
        </p>
      )}
      {!view && !failed && (
        <p className='text-muted-foreground text-sm'>{t('Loading...')}</p>
      )}
      <div className='grid gap-3 sm:grid-cols-2'>
        {rows.map((row) => {
          const link = view?.links.find(
            (link) => link.active && link.issuer === row.issuer
          )
          if (!link && !row.canBind) return null
          return (
            <div
              key={row.issuer}
              className='flex items-center justify-between gap-3 rounded-lg border p-3'
            >
              <div className='flex min-w-0 items-center gap-2'>
                <Link2 className='size-4 shrink-0' />
                <div className='min-w-0'>
                  <p className='text-sm font-medium'>{row.label}</p>
                  <p
                    className='text-muted-foreground truncate text-xs'
                    title={link?.subject}
                  >
                    {link?.subject ?? t('Not bound')}
                  </p>
                </div>
              </div>
              <Button
                size='sm'
                variant={link ? 'destructive' : 'outline'}
                disabled={busy}
                onClick={() => (link ? setTarget(link) : row.onBind())}
              >
                {link && <Unlink className='size-3' />}
                {link ? t('Unbind') : t('Bind')}
              </Button>
            </div>
          )
        })}
      </div>
      <ConfirmDialog
        open={target !== null}
        onOpenChange={(open) => {
          if (!open && !busy) setTarget(null)
        }}
        title={t('Unbind account')}
        desc={t(
          'This account will no longer sign you in. Other sessions will be signed out and identity history is retained.'
        )}
        confirmText={t('Unbind')}
        destructive
        isLoading={busy}
        handleConfirm={() => void unlink()}
      />
    </section>
  )
}
