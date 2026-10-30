/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { ErrorState } from '@/components/error-state'
import { LoadingState } from '@/components/loading-state'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'
import { getServerErrorMessage } from '@/lib/server-error-message'
import { useAuthStore } from '@/stores/auth-store'

import {
  getNativeConfiguration,
  getNativeKeys,
  nativeRequest as scopedNativeRequest,
  readNativeRoutingScope,
  type NativeRoutingScope,
  NATIVE_ROOT,
  parseNativeDocument,
  type NativeKeyList,
  type NativeModelRoute,
  type NativePriceDraft,
  type NativeUpstream,
  type NativeVendor,
} from '../native-api'

type Section = 'channels' | 'models' | 'vendors' | 'prices'
type Editor = {
  title: string
  path: string
  method: 'POST' | 'PUT'
  document: string
  previewPath?: string
  modelEditing?: boolean
  secret?: boolean
  readOnly?: boolean
}
const sections: Array<[Section, string]> = [
  ['channels', 'Channels'],
  ['models', 'Models'],
  ['vendors', 'Vendor'],
  ['prices', 'Pricing'],
]
const pretty = (value: unknown) => JSON.stringify(value, null, 2)
const nextId = (ids: number[]) =>
  ids.reduce((maximum, id) => Math.max(maximum, id), 0) + 1

/** Administrative edits always start from a server version and require CAS. */
export function NativeRoutingManagement({
  initialSection = 'channels',
}: {
  initialSection?: Section
}) {
  const { t } = useTranslation()
  const auth = useAuthStore((state) => state.auth)
  const scope = readNativeRoutingScope(auth)
  if (!scope) {
    return (
      <Alert variant='destructive' role='alert'>
        <AlertDescription>
          {t('Administrator access required')}
        </AlertDescription>
      </Alert>
    )
  }
  return (
    <NativeRoutingWorkspace
      key={`${scope.userId}:${scope.sessionId}:${scope.role}`}
      scope={scope}
      initialSection={initialSection}
    />
  )
}
function NativeRoutingWorkspace({
  initialSection,
  scope,
}: {
  initialSection: Section
  scope: NativeRoutingScope
}) {
  const nativeRequest = <T,>(
    method: 'GET' | 'POST' | 'PUT' | 'DELETE',
    path: string,
    data?: unknown
  ) => scopedNativeRequest<T>(scope, method, path, data)
  const { t } = useTranslation()
  const client = useQueryClient()
  const [section, setSection] = useState<Section>(initialSection)
  const [page, setPage] = useState(1)
  const [search, setSearch] = useState('')
  const [editor, setEditor] = useState<Editor | null>(null)
  const [secret, setSecret] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [preview, setPreview] = useState<{
    confirmation_digest: string
    [key: string]: unknown
  } | null>(null)
  const [confirmation, setConfirmation] = useState<string | null>(null)
  const [result, setResult] = useState<unknown>(null)
  const [keyChannel, setKeyChannel] = useState<number | null>(null)
  const [keys, setKeys] = useState<NativeKeyList | null>(null)
  const [keyLabel, setKeyLabel] = useState('')
  const [keySecret, setKeySecret] = useState('')
  const queryKey = [
    'native-routing-configuration',
    scope.userId,
    scope.sessionId,
    scope.role,
  ] as const
  const query = useQuery({
    queryKey,
    queryFn: () => getNativeConfiguration(scope),
    retry: false,
  })
  useEffect(
    () => () => {
      client.removeQueries({
        queryKey: [
          'native-routing-configuration',
          scope.userId,
          scope.sessionId,
          scope.role,
        ],
        exact: true,
      })
    },
    [client, scope.userId, scope.sessionId, scope.role]
  )
  const configuration = query.data

  function report(cause: unknown) {
    setError(
      getServerErrorMessage(cause) ??
        (cause instanceof Error ? cause.message : t('Failed to save'))
    )
  }
  function openEditor(value: Editor) {
    setEditor(value)
    setPreview(null)
    setSecret('')
    setError(null)
    setConfirmation(null)
    setResult(null)
  }
  async function refresh() {
    await Promise.all([
      query.refetch(),
      client.invalidateQueries({ queryKey: ['channels'] }),
      client.invalidateQueries({ queryKey: ['pricing'] }),
    ])
  }
  async function run(operation: () => Promise<unknown>, close = false) {
    if (busy) return
    setBusy(true)
    setError(null)
    setResult(null)
    try {
      const value = await operation()
      setResult(value)
      setConfirmation(null)
      if (close) {
        setEditor(null)
        setPreview(null)
        setSecret('')
      }
      await refresh()
    } catch (cause) {
      report(cause)
      setPreview(null)
      await refresh()
    } finally {
      setBusy(false)
    }
  }
  function channelEditor(channel?: NativeUpstream) {
    if (!configuration) return
    const id =
      channel?.id ??
      nextId(configuration.routing.upstreams.map((item) => item.id))
    const attachments = configuration.routing.routes.flatMap((route) =>
      route.targets
        .filter((target) => target.upstream_id === id)
        .map(
          ({
            id: _targetId,
            upstream_id: _upstreamId,
            enabled: _enabled,
            ...target
          }) => ({ route_id: route.id, ...target })
        )
    )
    openEditor({
      title: channel ? t('Edit Channel') : t('Create Channel'),
      method: channel ? 'PUT' : 'POST',
      path: `${NATIVE_ROOT}/channels${channel ? `/${id}` : ''}`,
      secret: !channel,
      document: pretty({
        expected_version: configuration.version,
        id,
        name: configuration.bindings.names[id] ?? '',
        endpoint: channel?.endpoint ?? '',
        protocol: channel?.protocol ?? 'openai_chat',
        enabled: channel?.enabled ?? false,
        contexts: configuration.bindings.contexts.filter(
          (context) => context.upstream_id === id
        ),
        attachments,
      }),
    })
  }
  function vendorEditor(vendor?: NativeVendor) {
    if (!configuration) return
    openEditor({
      title: vendor ? t('Edit') : t('Create'),
      method: vendor ? 'PUT' : 'POST',
      path: `${NATIVE_ROOT}/vendors${vendor ? `/${vendor.id}` : ''}`,
      document: pretty({
        expected_version: configuration.version,
        vendor: vendor ?? {
          id: nextId(
            Object.values(configuration.bindings.vendors).map((item) => item.id)
          ),
          name: '',
          description: '',
        },
      }),
    })
  }
  function modelEditor(route?: NativeModelRoute) {
    if (!configuration) return
    const id =
      route?.id ?? nextId(configuration.routing.routes.map((item) => item.id))
    const group =
      route?.group ?? configuration.routing.groups[0]?.name ?? 'default'
    const model = route?.model ?? ''
    openEditor({
      title: route ? t('Edit') : t('Add Model'),
      method: route ? 'PUT' : 'POST',
      path: `${NATIVE_ROOT}/models${route ? `/${id}` : ''}`,
      previewPath: `${NATIVE_ROOT}/models/preview`,
      modelEditing: !!route,
      document: pretty({
        expected_version: configuration.version,
        route: route ?? {
          id,
          model,
          group,
          enabled: false,
          speeds: ['standard'],
          targets: [],
          failover: {
            max_attempts: 1,
            retry_on: [],
            allow_replay_after_send: false,
          },
        },
        metadata: configuration.bindings.models[id] ?? {
          display_name: '',
          description: '',
          vendor_id: null,
        },
        prices: route
          ? configuration.prices.rules.filter(
              (price) =>
                price.model === route.model && price.group === route.group
            )
          : [
              {
                id: nextId(
                  configuration.prices.rules.map((price) => Number(price.id))
                ),
                model,
                group,
                speed: 'standard',
                rates: {
                  input: 0,
                  output: 0,
                  cache_read: 0,
                  cache_write: 0,
                  request: 0,
                },
                group_multiplier_ppm: 1_000_000,
                speed_multiplier_ppm: 1_000_000,
              },
            ],
        contexts: configuration.bindings.contexts.filter((context) =>
          route?.targets.some(
            (target) =>
              target.upstream_id === context.upstream_id &&
              target.upstream_model === context.upstream_model
          )
        ),
      }),
    })
  }
  function priceEditor() {
    if (!configuration) return
    openEditor({
      title: t('Pricing'),
      method: 'PUT',
      path: `${NATIVE_ROOT}/prices`,
      previewPath: `${NATIVE_ROOT}/prices/preview`,
      document: pretty({
        expected_version: configuration.version,
        prices: {
          ...configuration.prices,
          version: configuration.version.pricing + 1,
        },
      }),
    })
  }
  function draftEditor(draft?: NativePriceDraft) {
    if (!configuration) return
    const id =
      draft?.id ??
      nextId(
        Object.values(configuration.bindings.price_drafts).map(
          (item) => item.id
        )
      )
    openEditor({
      title: t('Save draft'),
      method: 'PUT',
      path: `${NATIVE_ROOT}/prices/drafts/${id}`,
      document: pretty({
        expected_version: configuration.version,
        name: draft?.name ?? '',
        prices: draft?.prices ?? {
          ...configuration.prices,
          version: configuration.version.pricing + 1,
        },
      }),
    })
  }
  async function previewEditor() {
    if (!editor?.previewPath || busy) return
    setBusy(true)
    setError(null)
    try {
      const body = parseNativeDocument(editor.document)
      const request =
        editor.modelEditing === undefined
          ? body
          : { editing: editor.modelEditing, model: body }
      const value = await nativeRequest<{
        confirmation_digest: string
        [key: string]: unknown
      }>('POST', editor.previewPath, request)
      if (
        typeof value.confirmation_digest !== 'string' ||
        !/^[0-9a-f]{64}$/.test(value.confirmation_digest)
      ) {
        throw new Error('The preview did not return a confirmation digest.')
      }
      setPreview(value)
    } catch (cause) {
      report(cause)
      setPreview(null)
    } finally {
      setBusy(false)
    }
  }
  function saveEditor() {
    if (!editor || (editor.previewPath && !preview)) return
    void run(async () => {
      const body = parseNativeDocument(editor.document)
      if (editor.secret) body.secret = secret
      if (preview) body.confirmation_digest = preview.confirmation_digest
      return nativeRequest(editor.method, editor.path, body)
    }, true)
  }
  function remove(path: string) {
    if (!configuration) return
    if (confirmation !== path) {
      setConfirmation(path)
      return
    }
    void run(() =>
      nativeRequest('DELETE', path, { expected_version: configuration.version })
    )
  }
  async function openKeys(id: number) {
    await run(async () => {
      const value = await getNativeKeys(scope, id)
      setKeys(value)
      setKeyChannel(id)
      setKeySecret('')
      setKeyLabel('')
      return null
    })
  }
  async function changeKey(operation: Record<string, unknown>) {
    if (!keys || keyChannel === null) return
    const expected = keys.version
    await run(async () => {
      await nativeRequest(
        'POST',
        `${NATIVE_ROOT}/channels/${keyChannel}/keys`,
        { expected_version: expected, ...operation }
      )
      setKeySecret('')
      setKeyLabel('')
      const current = await getNativeKeys(scope, keyChannel)
      setKeys(current)
      return null
    })
  }
  if (query.isLoading) return <LoadingState />
  if (query.isError || !configuration) {
    return (
      <ErrorState
        description={getServerErrorMessage(query.error) ?? t('Failed to load')}
        onRetry={() => void query.refetch()}
      />
    )
  }

  const matches = (...values: Array<string | number>) =>
    values.some((value) =>
      String(value).toLowerCase().includes(search.trim().toLowerCase())
    )
  const upstreams = configuration.routing.upstreams.filter((channel) =>
    matches(
      channel.id,
      configuration.bindings.names[channel.id] ?? '',
      channel.endpoint
    )
  )
  const models = configuration.routing.routes.filter((route) =>
    matches(
      route.id,
      route.model,
      route.group,
      configuration.bindings.models[route.id]?.display_name ?? ''
    )
  )
  const vendors = Object.values(configuration.bindings.vendors).filter(
    (vendor) => matches(vendor.id, vendor.name)
  )
  const prices = configuration.prices.rules.filter((price) =>
    matches(String(price.model), String(price.group))
  )
  const total =
    section === 'channels'
      ? upstreams.length
      : section === 'models'
        ? models.length
        : section === 'vendors'
          ? vendors.length
          : prices.length
  const start = (page - 1) * 25
  function deleteButton(path: string) {
    return (
      <Button
        type='button'
        variant={confirmation === path ? 'destructive' : 'outline'}
        className='min-h-11'
        disabled={busy}
        onClick={() => remove(path)}
      >
        {confirmation === path ? t('Confirm delete') : t('Delete')}
      </Button>
    )
  }
  return (
    <section className='min-w-0 space-y-4' aria-label={t('Configuration')}>
      <nav className='flex flex-wrap gap-2' aria-label={t('Configuration')}>
        {sections.map(([value, label]) => (
          <Button
            key={value}
            type='button'
            variant={section === value ? 'default' : 'outline'}
            className='min-h-11'
            disabled={busy}
            onClick={() => {
              setSection(value)
              setPage(1)
              setEditor(null)
              setPreview(null)
            }}
          >
            {t(label)}
          </Button>
        ))}
        <Button
          type='button'
          variant='ghost'
          className='min-h-11'
          disabled={busy}
          onClick={() => void refresh()}
        >
          {t('Refresh')}
        </Button>
      </nav>
      <label className='block space-y-2'>
        <span className='text-sm'>{t('Search')}</span>
        <Input
          className='min-h-11'
          value={search}
          onChange={(event) => {
            setSearch(event.target.value)
            setPage(1)
          }}
        />
      </label>
      {error && (
        <Alert variant='destructive' role='alert'>
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      )}
      {section === 'channels' && (
        <>
          <Button
            type='button'
            className='min-h-11'
            disabled={busy || !configuration.secret_storage_enabled}
            onClick={() => channelEditor()}
          >
            {t('Create Channel')}
          </Button>
          <div className='space-y-3'>
            {upstreams.slice(start, start + 25).map((channel) => (
              <div key={channel.id} className='min-w-0 space-y-2 border p-3'>
                <h3 className='font-medium [overflow-wrap:anywhere]'>
                  {configuration.bindings.names[channel.id] ??
                    `Channel ${channel.id}`}
                </h3>
                <p className='text-muted-foreground text-sm break-all'>
                  {channel.endpoint}
                </p>
                {configuration.bindings.discoveries[channel.id] && (
                  <details className='min-w-0'>
                    <summary className='cursor-pointer text-sm'>
                      {t('Models')}
                    </summary>
                    <time
                      className='text-muted-foreground text-xs'
                      dateTime={new Date(
                        configuration.bindings.discoveries[channel.id]
                          .observed_at * 1000
                      ).toISOString()}
                    >
                      {new Date(
                        configuration.bindings.discoveries[channel.id]
                          .observed_at * 1000
                      ).toLocaleString()}
                    </time>
                    <pre className='max-h-48 overflow-auto text-xs whitespace-pre-wrap'>
                      {configuration.bindings.discoveries[
                        channel.id
                      ].models.join('\n')}
                    </pre>
                  </details>
                )}
                <div className='flex flex-wrap gap-2'>
                  <Button
                    type='button'
                    variant='outline'
                    className='min-h-11'
                    disabled={busy}
                    onClick={() => channelEditor(channel)}
                  >
                    {t('Edit')}
                  </Button>
                  <Button
                    type='button'
                    variant='outline'
                    className='min-h-11'
                    disabled={busy}
                    onClick={() => void openKeys(channel.id)}
                  >
                    {t('API Keys')}
                  </Button>
                  <Button
                    type='button'
                    variant='outline'
                    className='min-h-11'
                    disabled={busy}
                    onClick={() =>
                      void run(() =>
                        nativeRequest('GET', `/api/channel/test/${channel.id}`)
                      )
                    }
                  >
                    {t('Test')}
                  </Button>
                  <Button
                    type='button'
                    variant='outline'
                    className='min-h-11'
                    disabled={busy}
                    onClick={() =>
                      void run(() =>
                        nativeRequest(
                          'POST',
                          `${NATIVE_ROOT}/channels/${channel.id}/models/synchronize`,
                          { expected_version: configuration.version }
                        )
                      )
                    }
                  >
                    {t('Sync')}
                  </Button>
                  {deleteButton(`${NATIVE_ROOT}/channels/${channel.id}`)}
                </div>
              </div>
            ))}
          </div>
        </>
      )}
      {section === 'models' && (
        <>
          <Button
            type='button'
            className='min-h-11'
            disabled={busy}
            onClick={() => modelEditor()}
          >
            {t('Add Model')}
          </Button>
          <div className='space-y-3'>
            {models.slice(start, start + 25).map((route) => (
              <div
                key={route.id}
                className='flex min-w-0 flex-wrap items-center justify-between gap-3 border p-3'
              >
                <div className='min-w-0'>
                  <h3 className='font-medium [overflow-wrap:anywhere]'>
                    {configuration.bindings.models[route.id]?.display_name ??
                      route.model}
                  </h3>
                  <p className='text-muted-foreground text-sm [overflow-wrap:anywhere]'>
                    {route.model} · {route.group} ·{' '}
                    {route.enabled ? t('Enabled') : t('Disabled')}
                  </p>
                </div>
                <div className='flex flex-wrap gap-2'>
                  <Button
                    type='button'
                    variant='outline'
                    className='min-h-11'
                    disabled={busy}
                    onClick={() => modelEditor(route)}
                  >
                    {t('Edit')}
                  </Button>
                  {deleteButton(`${NATIVE_ROOT}/models/${route.id}`)}
                </div>
              </div>
            ))}
          </div>
        </>
      )}
      {section === 'vendors' && (
        <>
          <Button
            type='button'
            className='min-h-11'
            disabled={busy}
            onClick={() => vendorEditor()}
          >
            {t('Create')}
          </Button>
          <div className='space-y-3'>
            {vendors.slice(start, start + 25).map((vendor) => (
              <div
                key={vendor.id}
                className='flex min-w-0 flex-wrap items-center justify-between gap-3 border p-3'
              >
                <div className='min-w-0'>
                  <h3 className='font-medium [overflow-wrap:anywhere]'>
                    {vendor.name}
                  </h3>
                  <p className='text-muted-foreground text-sm [overflow-wrap:anywhere]'>
                    {vendor.description}
                  </p>
                </div>
                <div className='flex flex-wrap gap-2'>
                  <Button
                    type='button'
                    variant='outline'
                    className='min-h-11'
                    disabled={busy}
                    onClick={() => vendorEditor(vendor)}
                  >
                    {t('Edit')}
                  </Button>
                  {deleteButton(`${NATIVE_ROOT}/vendors/${vendor.id}`)}
                </div>
              </div>
            ))}
          </div>
        </>
      )}
      {section === 'prices' && (
        <>
          <div className='flex flex-wrap gap-2'>
            <Button
              type='button'
              className='min-h-11'
              disabled={busy}
              onClick={priceEditor}
            >
              {t('Edit')}
            </Button>
            <Button
              type='button'
              variant='outline'
              className='min-h-11'
              disabled={busy}
              onClick={() => draftEditor()}
            >
              {t('Save draft')}
            </Button>
          </div>
          <pre className='max-h-80 overflow-auto border p-3 text-xs'>
            {pretty({
              ...configuration.prices,
              rules: prices.slice(start, start + 25),
            })}
          </pre>
          <div className='space-y-3'>
            {Object.values(configuration.bindings.price_drafts).map((draft) => (
              <div
                key={draft.id}
                className='flex min-w-0 flex-wrap items-center justify-between gap-3 border p-3'
              >
                <span className='font-medium [overflow-wrap:anywhere]'>
                  {draft.name}
                </span>
                <div className='flex flex-wrap gap-2'>
                  <Button
                    type='button'
                    variant='outline'
                    className='min-h-11'
                    disabled={busy}
                    onClick={() => draftEditor(draft)}
                  >
                    {t('Edit')}
                  </Button>
                  <Button
                    type='button'
                    variant='outline'
                    className='min-h-11'
                    disabled={
                      busy ||
                      draft.base_pricing_version !==
                        configuration.version.pricing
                    }
                    onClick={() =>
                      openEditor({
                        title: draft.name,
                        method: 'POST',
                        path: `${NATIVE_ROOT}/prices/drafts/${draft.id}/publish`,
                        previewPath: `${NATIVE_ROOT}/prices/drafts/${draft.id}/preview`,
                        document: pretty({
                          expected_version: configuration.version,
                        }),
                        readOnly: true,
                      })
                    }
                  >
                    {t('Preview')}
                  </Button>
                  {deleteButton(`${NATIVE_ROOT}/prices/drafts/${draft.id}`)}
                </div>
              </div>
            ))}
          </div>
        </>
      )}
      <div className='flex flex-wrap items-center justify-end gap-3'>
        <span className='text-sm'>
          {t('Total')}: {total}
        </span>
        <Button
          type='button'
          variant='outline'
          className='min-h-11'
          disabled={page === 1 || busy}
          onClick={() => setPage((value) => value - 1)}
        >
          {t('Previous')}
        </Button>
        <span className='text-sm'>
          {t('Page')} {page} / {Math.max(1, Math.ceil(total / 25))}
        </span>
        <Button
          type='button'
          variant='outline'
          className='min-h-11'
          disabled={page * 25 >= total || busy}
          onClick={() => setPage((value) => value + 1)}
        >
          {t('Next')}
        </Button>
      </div>
      {editor && (
        <form
          className='space-y-3 border p-4'
          onSubmit={(event) => {
            event.preventDefault()
            if (editor.previewPath && !preview) void previewEditor()
            else saveEditor()
          }}
        >
          <h3 className='font-medium'>{editor.title}</h3>
          <label className='block space-y-2'>
            <span className='text-sm'>{t('Configuration')}</span>
            <Textarea
              className='min-h-72 font-mono text-xs'
              value={editor.document}
              readOnly={editor.readOnly}
              disabled={busy}
              onChange={(event) => {
                setEditor({ ...editor, document: event.target.value })
                setPreview(null)
              }}
            />
          </label>
          {editor.secret && (
            <label className='block space-y-2'>
              <span className='text-sm'>{t('API key')}</span>
              <Input
                type='password'
                autoComplete='off'
                className='min-h-11'
                value={secret}
                disabled={busy}
                required
                onChange={(event) => setSecret(event.target.value)}
              />
            </label>
          )}
          {preview && (
            <pre
              className='max-h-80 overflow-auto border p-3 text-xs'
              aria-label={t('Preview')}
            >
              {pretty(preview)}
            </pre>
          )}
          <div className='flex flex-wrap gap-2'>
            <Button type='submit' className='min-h-11' disabled={busy}>
              {editor.previewPath
                ? preview
                  ? t('Confirm and apply')
                  : t('Preview')
                : t('Save')}
            </Button>
            <Button
              type='button'
              variant='outline'
              className='min-h-11'
              disabled={busy}
              onClick={() => {
                setEditor(null)
                setPreview(null)
                setSecret('')
              }}
            >
              {t('Cancel')}
            </Button>
          </div>
        </form>
      )}
      {keys && keyChannel !== null && (
        <section className='space-y-3 border p-4' aria-label={t('API Keys')}>
          <div className='flex flex-wrap items-center justify-between gap-2'>
            <h3 className='font-medium'>
              {t('API Keys')} · {keyChannel}
            </h3>
            <Button
              type='button'
              variant='outline'
              className='min-h-11'
              disabled={busy}
              onClick={() => void openKeys(keyChannel)}
            >
              {t('Refresh')}
            </Button>
            <Button
              type='button'
              variant='ghost'
              className='min-h-11'
              disabled={busy}
              onClick={() => {
                setKeys(null)
                setKeyChannel(null)
                setKeySecret('')
              }}
            >
              {t('Close')}
            </Button>
          </div>
          <p className='text-muted-foreground text-sm'>
            {t('ID')}: {keys.active_reference.id} /{' '}
            {keys.active_reference.revision}
          </p>
          {keys.keys.map((key) => {
            const active =
              key.reference.id === keys.active_reference.id &&
              key.reference.revision === keys.active_reference.revision
            const removeId = `key-${key.reference.id}-${key.reference.revision}`
            return (
              <div
                key={removeId}
                className='flex min-w-0 flex-wrap items-center justify-between gap-2 border p-3'
              >
                <span className='[overflow-wrap:anywhere]'>
                  {key.label} {active && `· ${t('Enabled')}`}
                </span>
                <div className='flex flex-wrap gap-2'>
                  <Button
                    type='button'
                    variant='outline'
                    className='min-h-11'
                    disabled={busy || active}
                    onClick={() =>
                      void changeKey({
                        operation: 'activate',
                        reference: key.reference,
                      })
                    }
                  >
                    {t('Enable')}
                  </Button>
                  <Button
                    type='button'
                    variant={
                      confirmation === removeId ? 'destructive' : 'outline'
                    }
                    className='min-h-11'
                    disabled={busy || active}
                    onClick={() => {
                      if (confirmation === removeId) {
                        void changeKey({
                          operation: 'remove',
                          reference: key.reference,
                        })
                      } else setConfirmation(removeId)
                    }}
                  >
                    {confirmation === removeId
                      ? t('Confirm delete')
                      : t('Delete')}
                  </Button>
                </div>
              </div>
            )
          })}
          <form
            className='space-y-3'
            onSubmit={(event) => {
              event.preventDefault()
              void changeKey({
                operation: 'add',
                label: keyLabel,
                secret: keySecret,
                activate: false,
              })
            }}
          >
            <label className='block space-y-2'>
              <span className='text-sm'>{t('Name')}</span>
              <Input
                className='min-h-11'
                value={keyLabel}
                maxLength={128}
                required
                disabled={busy || !configuration.secret_storage_enabled}
                onChange={(event) => setKeyLabel(event.target.value)}
              />
            </label>
            <label className='block space-y-2'>
              <span className='text-sm'>{t('API key')}</span>
              <Input
                className='min-h-11'
                type='password'
                autoComplete='off'
                value={keySecret}
                required
                disabled={busy || !configuration.secret_storage_enabled}
                onChange={(event) => setKeySecret(event.target.value)}
              />
            </label>
            <Button
              type='submit'
              className='min-h-11'
              disabled={busy || !configuration.secret_storage_enabled}
            >
              {t('Add')}
            </Button>
          </form>
        </section>
      )}
      {result !== null && (
        <pre
          className='max-h-80 overflow-auto border p-3 text-xs'
          role='status'
        >
          {pretty(result)}
        </pre>
      )}
    </section>
  )
}

export function NativeRoutingToggle() {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  return (
    <div className='space-y-4'>
      <Button
        type='button'
        variant='outline'
        className='min-h-11'
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
      >
        {t('Configuration')}
      </Button>
      {open && <NativeRoutingManagement />}
    </div>
  )
}
