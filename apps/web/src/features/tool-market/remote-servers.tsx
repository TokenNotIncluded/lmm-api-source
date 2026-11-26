/*
Copyright (C) 2026 LIghtJUNction
SPDX-License-Identifier: AGPL-3.0-or-later
*/
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useEffect, useMemo, useRef, useState } from 'react'

import { Button } from '@/components/ui/button'
import {
  Field,
  FieldDescription,
  FieldGroup,
  FieldLabel,
} from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { useAuthStore } from '@/stores/auth-store'

import { useMarketTranslation } from './provider-i18n'
import {
  prepareRemoteAuthorization,
  remoteServerAPI,
  takePendingInstallation,
  type RemoteInstallation,
} from './remote-server-api'

const selectClass =
  'border-input bg-background min-h-11 w-full rounded-md border px-3 text-sm'
type SavedServer = Pick<RemoteInstallation, 'id' | 'endpoint' | 'auth'>

export function RemoteServers({
  redirectURI,
  callback,
  clearCallback,
}: {
  redirectURI?: string
  callback: URLSearchParams | null
  clearCallback: () => void
}) {
  const { t } = useMarketTranslation()
  const oauthEnabled = Boolean(redirectURI)
  const userId = useAuthStore((state) => state.auth.user?.id)
  const sessionId = useAuthStore((state) => state.auth.session?.sid)
  const owner = useMemo(() => ({ userId, sessionId }), [userId, sessionId])
  const runtime = useMemo(() => remoteServerAPI(owner), [owner])
  const cache = useQueryClient()
  const [offset, setOffset] = useState(0)
  const list = useQuery({
    queryKey: ['tool-market', userId, 'remote-servers', offset],
    queryFn: () => runtime.list(offset),
    retry: false,
  })
  const [selected, setSelected] = useState<SavedServer>()
  const [endpoint, setEndpoint] = useState('')
  const [auth, setAuth] = useState<RemoteInstallation['auth']>('api_key')
  const [header, setHeader] = useState('Authorization')
  const [secret, setSecret] = useState('')
  const [clientID, setClientID] = useState('')
  const [scopes, setScopes] = useState('')
  const [connected, setConnected] = useState('')
  const [count, setCount] = useState<number>()
  const [error, setError] = useState(false)
  const [busy, setBusy] = useState(false)
  const lock = useRef(false)
  const handled = useRef<URLSearchParams | null>(null)
  const active = useRef(false)
  useEffect(() => {
    active.current = true
    return () => {
      active.current = false
    }
  }, [])

  useEffect(() => {
    if (!callback || handled.current === callback) return
    handled.current = callback
    clearCallback()
    lock.current = true
    setBusy(true)
    setError(false)
    void (async () => {
      try {
        if (!oauthEnabled) throw new Error('OAuth unavailable')
        const id = await takePendingInstallation(owner, callback)
        await runtime.finish(callback)
        setConnected(id)
      } catch {
        setError(true)
      } finally {
        lock.current = false
        setBusy(false)
      }
    })()
  }, [callback, clearCallback, oauthEnabled, owner, runtime])

  function edit(server?: SavedServer) {
    setSelected(server)
    setEndpoint(server?.endpoint ?? '')
    setAuth(server?.auth ?? 'api_key')
    setHeader('Authorization')
    setSecret('')
    setClientID('')
    setScopes('')
    setConnected('')
    setCount(undefined)
    setError(false)
  }

  async function connect() {
    if (lock.current || (auth === 'oauth' && !oauthEnabled)) return
    lock.current = true
    setBusy(true)
    setError(false)
    setCount(undefined)
    setConnected('')
    try {
      let id = selected?.id
      if (!id) {
        id = await runtime.register({
          endpoint: endpoint.trim(),
          auth,
          ...(auth === 'api_key' ? { header } : {}),
          ...(auth === 'oauth'
            ? { client_id: clientID.trim(), scopes: scopes.trim() }
            : {}),
        })
        setSelected({ id, endpoint: endpoint.trim(), auth })
        void cache.invalidateQueries({
          queryKey: ['tool-market', userId, 'remote-servers'],
        })
      }
      if (auth === 'oauth') {
        const start = await runtime.start(id)
        const url = await prepareRemoteAuthorization(owner, id, start)
        if (
          active.current &&
          useAuthStore.getState().auth.user?.id === userId &&
          useAuthStore.getState().auth.session?.sid === sessionId
        ) {
          window.location.assign(url)
        }
      } else {
        if (auth === 'api_key') {
          await runtime.key(id, secret)
          setSecret('')
        }
        setConnected(id)
      }
    } catch {
      setError(true)
    } finally {
      lock.current = false
      setBusy(false)
    }
  }

  async function discover(id: string) {
    if (lock.current) return
    lock.current = true
    setBusy(true)
    setError(false)
    setCount(undefined)
    try {
      setCount(await runtime.discover(id))
    } catch {
      setError(true)
    } finally {
      lock.current = false
      setBusy(false)
    }
  }

  return (
    <section className='bg-card mb-8 max-w-3xl space-y-5 rounded-xl border p-4 sm:p-6'>
      <h3 className='text-lg font-semibold'>
        {t('Connect a remote MCP server')}
      </h3>
      <form
        onSubmit={(event) => {
          event.preventDefault()
          void connect()
        }}
      >
        <fieldset disabled={busy}>
          <FieldGroup>
            <Field>
              <FieldLabel htmlFor='remote-mcp-url'>
                {t('Remote MCP endpoint')}
              </FieldLabel>
              <Input
                id='remote-mcp-url'
                type='url'
                required
                maxLength={2048}
                placeholder='https://example.com/mcp'
                disabled={!!selected}
                value={endpoint}
                onChange={(event) => {
                  setEndpoint(event.target.value)
                  setSecret('')
                }}
              />
              <FieldDescription>
                {t('Do not include API keys or tokens in the URL.')}
              </FieldDescription>
            </Field>
            <Field>
              <FieldLabel htmlFor='remote-mcp-auth'>
                {t('Authentication')}
              </FieldLabel>
              <select
                id='remote-mcp-auth'
                className={selectClass}
                value={auth}
                disabled={!!selected}
                onChange={(event) => {
                  setAuth(event.target.value as RemoteInstallation['auth'])
                  setSecret('')
                }}
              >
                <option value='api_key'>{t('API key')}</option>
                <option value='oauth' disabled={!oauthEnabled}>
                  {t('OAuth')}
                </option>
                <option value='none'>{t('No authentication')}</option>
              </select>
              {!oauthEnabled && (
                <FieldDescription>
                  {t('Provider OAuth is unavailable on this deployment.')}
                </FieldDescription>
              )}
            </Field>
            {auth === 'api_key' && (
              <>
                {!selected && (
                  <Field>
                    <FieldLabel htmlFor='remote-mcp-header'>
                      {t('Credential header')}
                    </FieldLabel>
                    <select
                      id='remote-mcp-header'
                      className={selectClass}
                      value={header}
                      onChange={(event) => {
                        setHeader(event.target.value)
                        setSecret('')
                      }}
                    >
                      <option value='Authorization'>{t('Bearer token')}</option>
                      <option value='X-Api-Key'>X-API-Key</option>
                      <option value='Api-Key'>Api-Key</option>
                    </select>
                  </Field>
                )}
                <Field>
                  <FieldLabel htmlFor='remote-mcp-secret'>
                    {t('Service credential')}
                  </FieldLabel>
                  <Input
                    id='remote-mcp-secret'
                    autoComplete='new-password'
                    type='password'
                    required
                    maxLength={8192}
                    value={secret}
                    onChange={(event) => setSecret(event.target.value)}
                  />
                </Field>
              </>
            )}
            {auth === 'oauth' && !selected && (
              <>
                <Field>
                  <FieldLabel htmlFor='remote-mcp-client'>
                    {t('Client ID')}
                  </FieldLabel>
                  <Input
                    id='remote-mcp-client'
                    required
                    maxLength={512}
                    value={clientID}
                    onChange={(event) => setClientID(event.target.value)}
                  />
                  <FieldDescription>
                    {t(
                      'Register a public OAuth client with the provider using this redirect URI.'
                    )}
                  </FieldDescription>
                  <code className='text-muted-foreground text-xs break-all'>
                    {redirectURI}
                  </code>
                </Field>
                <Field>
                  <FieldLabel htmlFor='remote-mcp-scopes'>
                    {t('Scopes')}
                  </FieldLabel>
                  <Input
                    id='remote-mcp-scopes'
                    maxLength={512}
                    value={scopes}
                    placeholder={t('Optional')}
                    onChange={(event) => setScopes(event.target.value)}
                  />
                </Field>
              </>
            )}
            <div className='flex flex-wrap gap-2'>
              <Button
                type='submit'
                disabled={
                  !endpoint.trim() ||
                  (auth === 'api_key' && !secret) ||
                  (auth === 'oauth' &&
                    (!oauthEnabled || (!selected && !clientID.trim())))
                }
              >
                {auth === 'oauth' ? t('Authorize') : t('Connect')}
              </Button>
              {selected && (
                <Button type='button' variant='outline' onClick={() => edit()}>
                  {t('New connection')}
                </Button>
              )}
            </div>
          </FieldGroup>
        </fieldset>
      </form>
      {connected && (
        <div className='flex flex-wrap items-center gap-3'>
          <span role='status' className='text-sm'>
            {t('Connection saved')}
          </span>
          <Button
            variant='outline'
            disabled={busy}
            onClick={() => void discover(connected)}
          >
            {t('Read tool definitions')}
          </Button>
        </div>
      )}
      {count !== undefined && (
        <p role='status' className='text-sm'>
          {t('Discovered tools: {{count}}', { count })}
        </p>
      )}
      {error && (
        <p role='alert' className='text-destructive text-sm'>
          {t('Connection failed')}
        </p>
      )}
      {list.isError && (
        <Button variant='outline' onClick={() => void list.refetch()}>
          {t('Retry')}
        </Button>
      )}
      {list.data && (list.data.items.length > 0 || offset > 0) && (
        <div className='space-y-2 border-t pt-4'>
          <h4 className='font-medium'>{t('Saved MCP servers')}</h4>
          {list.data.items
            .filter((item) => !item.release)
            .map((item) => (
              <div
                key={item.id}
                className='flex flex-wrap items-center justify-between gap-2'
              >
                <span className='min-w-0 flex-1 text-sm break-all'>
                  {item.endpoint}
                </span>
                <Button
                  variant='ghost'
                  disabled={
                    busy ||
                    !item.enabled ||
                    (item.auth === 'oauth' && !oauthEnabled)
                  }
                  onClick={() => edit(item)}
                >
                  {t('Authentication')}
                </Button>
                {item.id !== connected && (
                  <Button
                    variant='outline'
                    disabled={busy || !item.enabled}
                    onClick={() => void discover(item.id)}
                  >
                    {t('Read tool definitions')}
                  </Button>
                )}
              </div>
            ))}
          {(offset > 0 || list.data.has_more) && (
            <div className='flex gap-2'>
              <Button
                variant='ghost'
                disabled={!offset || busy}
                onClick={() => setOffset(offset - 20)}
              >
                {t('Previous')}
              </Button>
              <Button
                variant='ghost'
                disabled={!list.data.has_more || busy}
                onClick={() => setOffset(offset + 20)}
              >
                {t('Next')}
              </Button>
            </div>
          )}
        </div>
      )}
    </section>
  )
}
