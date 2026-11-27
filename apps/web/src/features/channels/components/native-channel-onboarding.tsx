/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useId, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Dialog } from '@/components/dialog'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { OfficialPriceEditor } from '@/features/system-settings/billing/official-price-editor'
import { getServerErrorMessage } from '@/lib/server-error-message'

import {
  NATIVE_ROOT,
  nativeRequest,
  type NativeConfiguration,
  type NativeRoutingScope,
} from '../native-api'
import { NativePendingRoute } from './native-pending-route'

const pageSize = 25

export function NativeChannelOnboarding({
  scope,
  configuration,
  channelId,
  initialModel,
  onRefresh,
  onClose,
}: {
  scope: NativeRoutingScope
  configuration: NativeConfiguration
  channelId: number
  initialModel?: string
  onRefresh: () => Promise<void>
  onClose: () => void
}) {
  const { t } = useTranslation()
  const saveHintId = useId()
  const [search, setSearch] = useState(initialModel ?? '')
  const [page, setPage] = useState(0)
  const [draft, setDraft] = useState<{
    version: number
    models: string[]
  } | null>(null)
  const [editing, setEditing] = useState<string | null>(initialModel ?? null)
  const [chosenModel, setChosenModel] = useState('')
  const [lastSync, setLastSync] = useState<{
    total: number
    added: Set<string>
  } | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const lock = useRef(false)
  const channel = configuration.routing.upstreams.find(
    (upstream) => upstream.id === channelId
  )
  const discovery = configuration.bindings.discoveries[channelId]
  const pending = new Set(discovery?.pending_models ?? [])
  const selected = new Set(draft?.models ?? pending)
  const changed =
    selected.size !== pending.size ||
    [...pending].some((model) => !selected.has(model))
  const stale = changed && draft?.version !== configuration.version.routing
  const groups = new Set(
    configuration.routing.groups
      .filter((group) => group.enabled)
      .map((group) => group.name)
  )
  const live = new Set<string>()
  const publicByUpstream = new Map<string, Set<string>>()
  for (const route of configuration.routing.routes) {
    for (const target of route.targets) {
      if (target.upstream_id !== channelId) continue
      const id = target.upstream_model
      const names = publicByUpstream.get(id) ?? new Set<string>()
      names.add(route.model)
      publicByUpstream.set(id, names)
      if (
        channel?.enabled &&
        route.enabled &&
        groups.has(route.group) &&
        target.enabled &&
        target.weight > 0 &&
        target.speeds.some((speed) => route.speeds.includes(speed))
      ) {
        live.add(id)
      }
    }
  }
  const publicModels = (id: string) => [...(publicByUpstream.get(id) ?? [id])]
  const knownModels = new Set([
    ...pending,
    ...(discovery?.models ?? []),
    ...live,
  ])
  const latestAdded = [...(lastSync?.added ?? [])].filter((id) =>
    knownModels.has(id)
  )
  const models = [...new Set([...latestAdded, ...knownModels])]
  const filtered = models.filter((model) =>
    model.toLowerCase().includes(search.trim().toLowerCase())
  )
  const currentPage = Math.min(
    page,
    Math.max(0, Math.ceil(filtered.length / pageSize) - 1)
  )
  const official = configuration.bindings.official_prices?.models ?? {}
  const missingOfficial = (model: string) =>
    !publicModels(model).some((id) => Object.hasOwn(official, id))
  const unpriced = [...pending].filter(
    (id) => !live.has(id) && missingOfficial(id)
  )
  const choices = editing ? publicModels(editing) : []
  const priced = choices.filter((name) => Object.hasOwn(official, name))
  const isNewModel = editing !== null && !publicByUpstream.has(editing)
  const savedModel = choices.includes(chosenModel)
    ? chosenModel
    : choices.length === 1
      ? choices[0]
      : priced.length === 1
        ? priced[0]
        : ''

  const publicModel = isNewModel ? chosenModel.trim() || editing : savedModel

  async function update(sync: boolean) {
    if (lock.current || stale || (sync ? changed : !changed)) return
    lock.current = true
    setBusy(true)
    setError('')
    try {
      const path = `${NATIVE_ROOT}/channels/${channelId}/models/`
      if (sync) {
        const receipt = await nativeRequest<{ models: string[] }>(
          scope,
          'POST',
          `${path}synchronize`,
          { expected_version: configuration.version }
        )
        const returned = new Set(receipt.models)
        setLastSync({
          total: returned.size,
          added: new Set([...returned].filter((id) => !knownModels.has(id))),
        })
        setPage(0)
        setSearch('')
      } else {
        await nativeRequest(scope, 'PUT', `${path}pending`, {
          expected_version: configuration.version,
          models: [...selected],
        })
      }
      await onRefresh()
      setDraft(null)
    } catch (cause) {
      setError(getServerErrorMessage(cause) ?? t('Failed to save'))
    } finally {
      lock.current = false
      setBusy(false)
    }
  }

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !busy) onClose()
      }}
      footer={
        <Button
          type='button'
          variant='outline'
          disabled={busy}
          onClick={onClose}
        >
          {t('Close')}
        </Button>
      }
      title={t('Model onboarding')}
      description={
        configuration.bindings.names[channelId] ??
        t('Channel {{id}}', { id: channelId })
      }
    >
      <div className='space-y-5'>
        <div className='flex flex-wrap gap-2'>
          <Button
            type='button'
            disabled={!channel || busy || changed}
            onClick={() => void update(true)}
          >
            {t('Sync models')}
          </Button>
          <Button
            type='button'
            variant='outline'
            disabled={busy || stale || !changed}
            aria-describedby={!busy && !changed ? saveHintId : undefined}
            onClick={() => void update(false)}
          >
            {t('Save pending models')}
          </Button>
          {changed && (
            <Button
              type='button'
              variant='ghost'
              disabled={busy}
              onClick={() => setDraft(null)}
            >
              {t('Discard changes')}
            </Button>
          )}
        </div>
        {!busy && !changed && (
          <p id={saveHintId} className='text-muted-foreground text-sm'>
            {t('Change the pending selection to save it.')}
          </p>
        )}
        {error && (
          <p role='alert' className='text-destructive text-sm'>
            {error}
          </p>
        )}
        {stale && (
          <p role='alert'>
            {t(
              'Configuration changed. Your draft is preserved. Reload the editor before saving.'
            )}
          </p>
        )}
        {lastSync && (
          <p role='status' className='text-sm'>
            {t('Last sync: {{total}} returned, {{added}} newly discovered.', {
              total: lastSync.total,
              added: lastSync.added.size,
            })}
          </p>
        )}
        <p className='text-muted-foreground text-sm'>
          {t(
            'Channel models: {{live}} enabled, {{unpriced}} pending without an official base price.',
            { live: live.size, unpriced: unpriced.length }
          )}
        </p>
        <Input
          aria-label={t('Search models')}
          placeholder={t('Search models')}
          value={search}
          onChange={(event) => {
            setSearch(event.target.value)
            setPage(0)
          }}
        />
        <ul className='divide-y'>
          {filtered
            .slice(currentPage * pageSize, (currentPage + 1) * pageSize)
            .map((model) => (
              <li
                key={model}
                className='flex min-h-12 flex-wrap items-center gap-3 py-2'
              >
                {live.has(model) ? (
                  <span className='min-w-0 flex-1 font-mono text-sm break-all'>
                    {model}
                  </span>
                ) : (
                  <label className='flex min-w-0 flex-1 items-center gap-2'>
                    <input
                      type='checkbox'
                      aria-label={model}
                      checked={selected.has(model)}
                      disabled={busy || stale}
                      onChange={(event) => {
                        const next = new Set(selected)
                        if (event.target.checked) next.add(model)
                        else next.delete(model)
                        setDraft({
                          version: configuration.version.routing,
                          models: [...next],
                        })
                      }}
                    />
                    <span className='font-mono text-sm break-all'>{model}</span>
                  </label>
                )}
                {lastSync?.added.has(model) && (
                  <span className='text-primary text-xs'>
                    {t('Newly discovered')}
                  </span>
                )}
                <span className='text-muted-foreground text-xs'>
                  {live.has(model)
                    ? t('Enabled')
                    : pending.has(model)
                      ? missingOfficial(model)
                        ? t('Pending pricing')
                        : t('Pending activation')
                      : selected.has(model)
                        ? t('Save pending models')
                        : t('Select to add to pending models')}
                </span>
                {!live.has(model) && missingOfficial(model) && (
                  <span className='text-muted-foreground text-xs break-all'>
                    {t('Missing official base price: {{models}}', {
                      models: publicModels(model).join(', '),
                    })}
                  </span>
                )}
                {pending.has(model) && !live.has(model) && (
                  <Button
                    type='button'
                    variant='outline'
                    size='sm'
                    disabled={busy || changed}
                    onClick={() => {
                      setEditing(editing === model ? null : model)
                      setChosenModel('')
                    }}
                  >
                    {t('Pricing')}
                  </Button>
                )}
              </li>
            ))}
        </ul>
        {!filtered.length && (
          <p className='text-muted-foreground text-sm'>
            {t('No models available')}
          </p>
        )}
        {filtered.length > pageSize && (
          <div className='flex items-center gap-3'>
            <Button
              type='button'
              variant='outline'
              disabled={currentPage === 0}
              onClick={() => setPage(currentPage - 1)}
            >
              {t('Previous')}
            </Button>
            <span>
              {currentPage + 1} / {Math.ceil(filtered.length / pageSize)}
            </span>
            <Button
              type='button'
              variant='outline'
              disabled={(currentPage + 1) * pageSize >= filtered.length}
              onClick={() => setPage(currentPage + 1)}
            >
              {t('Next')}
            </Button>
          </div>
        )}
        {channel &&
          editing &&
          pending.has(editing) &&
          !live.has(editing) &&
          !changed && (
            <section className='space-y-4 border-t pt-4' aria-label={editing}>
              <h3 className='font-mono text-sm font-medium break-all'>
                {editing}
              </h3>
              {isNewModel && (
                <details>
                  <summary className='cursor-pointer text-sm'>
                    {t('Model name')}
                  </summary>
                  <label className='mt-2 grid gap-1 text-sm'>
                    {t('Model ID')}
                    <Input
                      value={chosenModel}
                      placeholder={editing}
                      maxLength={256}
                      onChange={(event) => setChosenModel(event.target.value)}
                    />
                  </label>
                </details>
              )}
              {choices.length > 1 && (
                <label className='grid gap-1 text-sm'>
                  {t('Model ID')}
                  <select
                    className='border-input bg-background rounded-md border px-3 py-2'
                    value={publicModel}
                    onChange={(event) => setChosenModel(event.target.value)}
                  >
                    <option value='' disabled>
                      {t('Choose a model')}
                    </option>
                    {choices.map((id) => (
                      <option key={id} value={id}>
                        {id}
                      </option>
                    ))}
                  </select>
                </label>
              )}
              {publicModel && (
                <OfficialPriceEditor
                  scope={scope}
                  configuration={configuration}
                  model={publicModel}
                  channelId={channelId}
                  {...(isNewModel && { upstreamModel: editing })}
                  onPublished={onRefresh}
                />
              )}
              {publicModel && Object.hasOwn(official, publicModel) && (
                <NativePendingRoute
                  key={JSON.stringify([
                    channelId,
                    publicModel,
                    editing,
                    configuration.version,
                  ])}
                  scope={scope}
                  configuration={configuration}
                  channel={channel}
                  model={publicModel}
                  upstreamModel={editing}
                  onPublished={onRefresh}
                />
              )}
            </section>
          )}
      </div>
    </Dialog>
  )
}
