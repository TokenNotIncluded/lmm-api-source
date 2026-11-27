/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useId, useMemo, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Dialog } from '@/components/dialog'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { OfficialBatchPriceReview } from '@/features/system-settings/billing/official-batch-price-review'
import { OfficialPriceEditor } from '@/features/system-settings/billing/official-price-editor'
import { getServerErrorMessage } from '@/lib/server-error-message'

import { nativeConfiguredPrice } from '../lib/native-price-display'
import {
  NATIVE_ROOT,
  nativeRequest,
  type NativeConfiguration,
  type NativeRoutingScope,
} from '../native-api'
import { NativeIntakeReview } from './native-intake-review'
import { groupIntake, intakeIndex } from './native-intake-routes'
import { microsToUsd } from './native-model-draft'
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
    removed: Set<string>
  } | null>(null)
  const [busy, setBusy] = useState(false)
  const [reviewBusy, setReviewBusy] = useState(false)
  const [reviewIds, setReviewIds] = useState<Set<string>>(new Set())
  const [overrides, setOverrides] = useState<Record<string, string>>({})
  const [details, setDetails] = useState<string | null>(null)
  const [filter, setFilter] = useState('all')
  const [panel, setPanel] = useState<'prices' | 'routes' | null>(null)
  const index = useMemo(() => intakeIndex(configuration), [configuration])
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
    ...publicByUpstream.keys(),
  ])
  const latestAdded = [...(lastSync?.added ?? [])].filter((id) =>
    knownModels.has(id)
  )
  const models = [
    ...new Set([...latestAdded, ...knownModels, ...(lastSync?.removed ?? [])]),
  ]
  const observed = new Set(discovery?.models ?? [])
  function rowInfo(id: string) {
    const names = publicModels(id)
    const override = Object.hasOwn(overrides, id) ? overrides[id] : undefined
    const pair = {
      upstream: id,
      model: (override ?? (names.length === 1 ? names[0] : '')).trim(),
    }
    const states =
      channel && pair.model
        ? [...groups].map((group) => ({
            group,
            state: groupIntake(configuration, index, channel, pair, group),
          }))
        : []
    return {
      names,
      pair,
      states,
      absent: Boolean(discovery && !observed.has(id)),
      missing: states
        .filter(
          ({ state }) => !state.prices.length || state.prices.some((p) => !p)
        )
        .map(({ group }) => group),
      ready: states.some(({ state }) => !state.done && !state.reasons.length),
    }
  }
  const reviewModels = [...reviewIds].map((id) => rowInfo(id).pair)
  const resolved = reviewModels.every((pair) => pair.model.length > 0)
  const filters = [
    ['all', t('All models')],
    ['new', t('Newly discovered')],
    ['removed', t('Absent from upstream catalog')],
    ['missing', t('Missing price')],
    ['ready', t('Ready for route preview')],
  ]
  const filtered = models.filter((model) => {
    if (!model.toLowerCase().includes(search.trim().toLowerCase())) return false
    if (filter === 'new') return lastSync?.added.has(model)
    if (filter === 'removed') return Boolean(discovery && !observed.has(model))
    if (filter === 'all') return true
    const info = rowInfo(model)
    return filter === 'ready'
      ? info.ready
      : info.missing.length > 0 || !info.states[0]?.state.base
  })
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
    if (lock.current || reviewBusy || stale || (sync ? changed : !changed))
      return
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
          added: new Set([...returned].filter((id) => !observed.has(id))),
          removed: new Set([...observed].filter((id) => !returned.has(id))),
        })
        setPage(0)
        setSearch('')
        setPanel(null)
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
        if (!open && !busy && !reviewBusy) onClose()
      }}
      footer={
        <Button
          type='button'
          variant='outline'
          disabled={busy || reviewBusy}
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
            disabled={!channel || busy || changed || reviewBusy}
            onClick={() => void update(true)}
          >
            {t('Sync models')}
          </Button>
          <Button
            type='button'
            variant='outline'
            disabled={busy || stale || !changed || reviewBusy}
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
            })}{' '}
            {t('Removed since last sync: {{count}}.', {
              count: lastSync.removed.size,
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
        <div className='flex flex-wrap items-end gap-3'>
          <label className='grid gap-1 text-sm'>
            {t('Intake status')}
            <select
              className='border-input bg-background rounded-md border p-2'
              value={filter}
              onChange={(e) => {
                setFilter(e.target.value)
                setPage(0)
              }}
            >
              {filters.map(([value, label]) => (
                <option key={value} value={value}>
                  {label}
                </option>
              ))}
            </select>
          </label>
          <Button
            type='button'
            variant='outline'
            disabled={
              !reviewModels.length || busy || changed || reviewBusy || !resolved
            }
            onClick={() => {
              setEditing(null)
              setPanel('prices')
            }}
          >
            {t('Sync missing prices from models.dev')}
          </Button>
          <Button
            type='button'
            variant='outline'
            disabled={
              !reviewModels.length || busy || changed || reviewBusy || !resolved
            }
            onClick={() => {
              setEditing(null)
              setPanel('routes')
            }}
          >
            {t('Review selected routes')}
          </Button>
          {reviewModels.length > 0 && (
            <span className='text-muted-foreground text-sm'>
              {t('Selected')}: {reviewModels.length}
            </span>
          )}
        </div>
        <div className='overflow-x-auto'>
          <table className='w-full text-start text-sm'>
            <thead className='text-muted-foreground border-b text-xs'>
              <tr>
                {[
                  t('Review'),
                  t('Model'),
                  t('Pending'),
                  t('Status'),
                  t('Group price rules'),
                ].map((label, i) => (
                  <th
                    scope='col'
                    className='px-2 py-2 text-start font-medium'
                    key={i}
                  >
                    {label}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {filtered
                .slice(currentPage * pageSize, (currentPage + 1) * pageSize)
                .map((model) => {
                  const info = rowInfo(model)
                  const selectedForReview = reviewIds.has(model)
                  const summary = info.absent
                    ? t('Absent from upstream catalog')
                    : live.has(model)
                      ? t('Enabled')
                      : pending.has(model)
                        ? missingOfficial(model)
                          ? t('Pending pricing')
                          : t('Pending activation')
                        : selected.has(model)
                          ? t('Save pending models')
                          : t('Select to add to pending models')
                  return (
                    <tr
                      key={model}
                      data-intake-model={model}
                      className='border-b align-top'
                    >
                      <td className='p-2'>
                        <input
                          type='checkbox'
                          aria-label={t('Review model {{model}}', { model })}
                          checked={selectedForReview}
                          disabled={busy || reviewBusy}
                          onChange={(e) => {
                            const next = new Set(reviewIds)
                            if (e.target.checked) next.add(model)
                            else next.delete(model)
                            setPanel(null)
                            setReviewIds(next)
                          }}
                        />
                      </td>
                      <td className='min-w-36 p-2'>
                        <span className='font-mono break-all'>{model}</span>
                        {lastSync?.added.has(model) && (
                          <p className='text-primary text-xs'>
                            {t('Newly discovered')}
                          </p>
                        )}
                        {selectedForReview &&
                          (info.names.length > 1 ? (
                            <label className='mt-2 grid gap-1 text-xs'>
                              {t('Public model ID')}
                              <select
                                className='border-input rounded border p-1'
                                value={info.pair.model}
                                disabled={reviewBusy}
                                onChange={(e) => {
                                  setPanel(null)
                                  setOverrides({
                                    ...overrides,
                                    [model]: e.target.value,
                                  })
                                }}
                              >
                                <option value=''>{t('Choose a model')}</option>
                                {info.names.map((id) => (
                                  <option key={id}>{id}</option>
                                ))}
                              </select>
                            </label>
                          ) : (
                            !publicByUpstream.has(model) && (
                              <label className='mt-2 grid gap-1 text-xs'>
                                {t('Public model ID')}
                                <Input
                                  value={info.pair.model}
                                  disabled={reviewBusy}
                                  maxLength={256}
                                  onChange={(e) => {
                                    setPanel(null)
                                    setOverrides({
                                      ...overrides,
                                      [model]: e.target.value,
                                    })
                                  }}
                                />
                              </label>
                            )
                          ))}
                      </td>
                      <td className='p-2'>
                        {!live.has(model) && (
                          <input
                            type='checkbox'
                            aria-label={model}
                            checked={selected.has(model)}
                            disabled={
                              busy ||
                              stale ||
                              reviewBusy ||
                              (info.absent && !pending.has(model))
                            }
                            onChange={(event) => {
                              const next = new Set(selected)
                              if (event.target.checked) next.add(model)
                              else next.delete(model)
                              setPanel(null)
                              setDraft({
                                version: configuration.version.routing,
                                models: [...next],
                              })
                            }}
                          />
                        )}
                      </td>
                      <td className='min-w-32 p-2'>
                        <p>{summary}</p>
                        {missingOfficial(model) && !live.has(model) && (
                          <p className='text-muted-foreground text-xs'>
                            {t('Missing official base price: {{models}}', {
                              models: info.pair.model || info.names.join(', '),
                            })}
                          </p>
                        )}
                        {!info.pair.model && (
                          <p className='text-xs'>
                            {t('Choose a public model ID.')}
                          </p>
                        )}
                        {info.ready && (
                          <p className='text-xs'>
                            {t('Ready for route preview')}
                          </p>
                        )}
                        {pending.has(model) && !live.has(model) && (
                          <Button
                            type='button'
                            variant='ghost'
                            size='sm'
                            disabled={busy || changed || reviewBusy}
                            onClick={() => {
                              setPanel(null)
                              setEditing(editing === model ? null : model)
                              setChosenModel('')
                            }}
                          >
                            {t('Pricing')}
                          </Button>
                        )}
                      </td>
                      <td className='min-w-48 p-2'>
                        {info.missing.length > 0 && (
                          <p className='text-xs'>
                            {t('Missing group prices: {{groups}}', {
                              groups: info.missing.join(', '),
                            })}
                          </p>
                        )}
                        <Button
                          type='button'
                          size='sm'
                          variant='ghost'
                          aria-expanded={details === model}
                          onClick={() =>
                            setDetails(details === model ? null : model)
                          }
                        >
                          {t('Group prices and blockers')}
                        </Button>
                        {details === model && (
                          <div className='space-y-2 text-xs'>
                            <p>
                              {t('Price source')}{' '}
                              {info.states[0]?.state.base?.source ??
                                t('Not recorded')}{' '}
                              · {t('Pricing version')}{' '}
                              {configuration.version.pricing}
                            </p>
                            {info.states.map(({ group, state }) => (
                              <div key={group}>
                                <p className='font-medium'>
                                  {group} · ×
                                  {microsToUsd(
                                    Object.hasOwn(
                                      configuration.bindings.official_prices
                                        ?.group_multipliers_ppm ?? {},
                                      group
                                    )
                                      ? configuration.bindings.official_prices!
                                          .group_multipliers_ppm[group]
                                      : 1_000_000
                                  )}
                                </p>
                                {state.base &&
                                  (state.base.tiered ? (
                                    <p>{t('Tiered pricing')}</p>
                                  ) : (
                                    <p>
                                      {t('Base price (1×)')} ·{' '}
                                      {t('Input price (USD / 1M tokens)')}{' '}
                                      {microsToUsd(state.base.rates.input)}
                                      {' · '}
                                      {t('Output price (USD / 1M tokens)')}{' '}
                                      {microsToUsd(state.base.rates.output)}
                                    </p>
                                  ))}
                                {state.prices
                                  .filter(
                                    (r): r is Record<string, unknown> => !!r
                                  )
                                  .map((rule) => (
                                    <p key={String(rule.id)}>
                                      {String(rule.speed)} ·{' '}
                                      {t(
                                        'Effective input price (USD / 1M tokens)'
                                      )}{' '}
                                      {nativeConfiguredPrice(rule, 'input') ??
                                        '—'}
                                      {' · '}
                                      {t(
                                        'Effective output price (USD / 1M tokens)'
                                      )}{' '}
                                      {nativeConfiguredPrice(rule, 'output') ??
                                        '—'}
                                    </p>
                                  ))}
                                <p>
                                  {state.reasons.length
                                    ? state.reasons
                                        .map((reason) => t(reason))
                                        .join(' · ')
                                    : state.done
                                      ? t('Already published')
                                      : t(
                                          'Group price will be generated in the route preview.'
                                        )}
                                </p>
                              </div>
                            ))}
                          </div>
                        )}
                      </td>
                    </tr>
                  )
                })}
            </tbody>
          </table>
        </div>
        {panel === 'prices' && resolved && (
          <OfficialBatchPriceReview
            key={JSON.stringify(reviewModels)}
            scope={scope}
            configuration={configuration}
            models={[...new Set(reviewModels.map((m) => m.model))]}
            channelId={channelId}
            onPublished={async () => {
              await onRefresh()
              setPanel('routes')
            }}
            onClose={() => setPanel(null)}
          />
        )}
        {panel === 'routes' && channel && resolved && (
          <NativeIntakeReview
            key={JSON.stringify([configuration.version, reviewModels])}
            scope={scope}
            configuration={configuration}
            channel={channel}
            models={reviewModels}
            onBusy={setReviewBusy}
            onRefresh={onRefresh}
            onClose={() => setPanel(null)}
          />
        )}
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
