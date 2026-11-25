/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.

For commercial licensing, please contact support@quantumnous.com
*/
import { useCallback, useEffect, useReducer, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import {
  humanGrantReadErrorKey,
  readHumanGrantSettings,
  writeHumanGrantSettings,
  type HumanGrantSettings,
} from '@/features/auth/lib/humangrant'
import {
  createHumanGrantSessionScope,
  type HumanGrantIdentity,
} from '@/features/auth/lib/humangrant-session'
import { useAuthStore } from '@/stores/auth-store'

function currentIdentity() {
  const { user, session } = useAuthStore.getState().auth
  return { userId: user?.id, sessionId: session?.sid }
}

function renewHumanGrantScope(
  _previous: ReturnType<typeof createHumanGrantSessionScope>,
  identity: HumanGrantIdentity
) {
  return createHumanGrantSessionScope(identity, currentIdentity)
}

export function HumanGrantSection({
  readOnly = false,
}: { readOnly?: boolean } = {}) {
  const userId = useAuthStore((state) => state.auth.user?.id)
  const sessionId = useAuthStore((state) => state.auth.session?.sid)
  // A new session gets new drafts, proofs, pending requests and save guards.
  return (
    <HumanGrantSettingsForm
      key={JSON.stringify([userId, sessionId])}
      readOnly={readOnly}
      userId={userId}
      sessionId={sessionId}
    />
  )
}

function HumanGrantSettingsForm({
  readOnly,
  userId,
  sessionId,
}: {
  readOnly: boolean
  userId: number | undefined
  sessionId: string | undefined
}) {
  const { t } = useTranslation()
  const [settings, setSettings] = useState<HumanGrantSettings | null>(null)
  const [busy, setBusy] = useState(true)
  const [message, setMessage] = useState('')
  const request = useRef<AbortController | null>(null)
  // Ref guards against a second submission before React renders disabled.
  const saving = useRef<AbortController | null>(null)
  const [scope, renewScope] = useReducer(
    renewHumanGrantScope,
    { userId, sessionId },
    (identity) => createHumanGrantSessionScope(identity, currentIdentity)
  )

  const loadSettings = useCallback(() => {
    if (saving.current) return
    request.current?.abort()
    const active = new AbortController()
    request.current = active
    const timer = setTimeout(() => active.abort(), 10_000)
    return readHumanGrantSettings(active.signal, scope.fetch)
      .then((current) => {
        scope.check()
        active.signal.throwIfAborted()
        if (request.current === active && !active.signal.aborted) {
          setSettings(current)
        }
      })
      .catch((error: unknown) => {
        if (request.current === active) {
          setMessage(humanGrantReadErrorKey(error))
        }
      })
      .finally(() => {
        clearTimeout(timer)
        if (request.current === active) {
          request.current = null
          setBusy(false)
        }
      })
  }, [scope])

  const reload = () => {
    if (saving.current) return
    setBusy(true)
    setSettings(null)
    setMessage('')
    return loadSettings()
  }

  const onIdentityChange = useCallback(() => {
    try {
      scope.check()
    } catch {
      request.current?.abort()
      request.current = null
      saving.current = null
      setSettings(null)
      setMessage('')
      setBusy(true)
      // Batched A -> B -> A changes still need a fresh proof and scope.
      renewScope({ userId, sessionId })
    }
  }, [scope, userId, sessionId])

  useEffect(() => {
    // Subscription observes A -> B -> A transitions even before React renders.
    // An invalidated request must never become valid when an account returns.
    const unsubscribe = useAuthStore.subscribe(onIdentityChange)
    void loadSettings()
    return () => {
      unsubscribe()
      request.current?.abort()
      request.current = null
      saving.current = null
    }
  }, [loadSettings, onIdentityChange])

  async function save() {
    if (saving.current || busy || settings === null) return
    request.current?.abort()
    const active = new AbortController()
    saving.current = active
    request.current = active
    const timer = setTimeout(() => active.abort(), 10_000)
    setBusy(true)
    setMessage('')
    try {
      const current = await writeHumanGrantSettings(
        settings,
        active.signal,
        scope.fetch
      )
      scope.check()
      active.signal.throwIfAborted()
      if (request.current === active && !active.signal.aborted) {
        setSettings(current)
        setMessage('HumanGrant settings saved.')
      }
    } catch {
      if (request.current === active) {
        // UNKNOWN writes require reconciliation; never retry without a new proof.
        setSettings(null)
        setMessage(
          'The save result is unknown. Reload HumanGrant settings before making another change.'
        )
      }
    } finally {
      clearTimeout(timer)
      if (saving.current === active) saving.current = null
      if (request.current === active) {
        request.current = null
        setBusy(false)
      }
    }
  }

  if (readOnly) {
    return (
      <section className='space-y-3' aria-busy={busy}>
        <p>
          {t('Enable HumanGrant')} ·{' '}
          {settings
            ? settings.enabled
              ? t('Enabled')
              : t('Disabled')
            : t('Unavailable')}
        </p>
        <p className='text-muted-foreground text-sm'>
          {t('Source')} · <code>/api/user/auth/humangrant/settings</code> ·{' '}
          {t('Revision')} · {t('Unavailable')}
        </p>
        {message && <p role='status'>{t(message)}</p>}
        <Button
          type='button'
          variant='outline'
          disabled={busy}
          onClick={() => void reload()}
        >
          {t('Reload HumanGrant settings')}
        </Button>
        <a className='block underline' href='/human/grants'>
          {t('Manage my AI grants')}
        </a>
      </section>
    )
  }

  return (
    <section className='flex flex-col gap-5' aria-busy={busy}>
      <h2 className='text-lg font-medium'>HumanGrant</h2>
      <p className='text-muted-foreground'>
        {t(
          'HumanGrant lets an AI act with approved, limited access. It does not share your login session.'
        )}
      </p>
      <label className='flex min-h-11 cursor-pointer items-center gap-3'>
        <input
          type='checkbox'
          className='size-5'
          checked={settings?.enabled === true}
          disabled={busy || settings === null}
          onChange={(event) => {
            const enabled = event.currentTarget.checked
            setSettings((current) =>
              current === null ? null : { ...current, enabled }
            )
            setMessage('')
          }}
        />
        {t('Enable HumanGrant')}
      </label>
      <p className='text-muted-foreground text-sm'>
        {t(
          'Enabled by default. Turning it off stops new requests and invalidates existing AI grants. Turning it on does not restore old grants.'
        )}
      </p>
      <div className='flex flex-wrap gap-3'>
        <Button
          type='button'
          onClick={save}
          disabled={busy || settings === null}
        >
          {t('Save')}
        </Button>
        <Button
          type='button'
          variant='outline'
          disabled={busy}
          onClick={() => {
            void reload()
          }}
        >
          {t('Reload HumanGrant settings')}
        </Button>
      </div>
      <p role='status' aria-live='polite'>
        {message ? t(message) : null}
      </p>
      <a className='underline' href='/human/grants'>
        {t('Manage my AI grants')}
      </a>
    </section>
  )
}
