/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useMutation } from '@tanstack/react-query'
import { ChevronDown } from 'lucide-react'
import {
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Combobox } from '@/components/ui/combobox'
import { Input } from '@/components/ui/input'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { getServerErrorMessage } from '@/lib/server-error-message'

import {
  detectUpstreamApi,
  presetProtocolEndpoint,
  resolveNativeProvider,
} from '../lib/native-provider'
import {
  commonChannelTargetValue,
  nativeProtocolLabel,
  nativeProtocols,
  readChannelDraft,
  toggleChannelAttachment,
  updateChannelAttachment,
  updateNativeContextDestination,
  type NativeChannelDraft,
} from '../lib/native-routing-form'
import {
  parseNativeDocument,
  stringifyNativeDocument,
  type NativeConfiguration,
} from '../native-api'
import { NativeChannelModelsFields } from './native-channel-models-fields'
import { NativeProviderLogo } from './native-provider-logo'

export function NativeChannelTagField({
  document,
  disabled,
  onChange,
}: {
  document: string
  disabled: boolean
  onChange: (document: string) => void
}) {
  const { t } = useTranslation()
  const draft = readChannelDraft(document)
  if (!draft) return null
  return (
    <label className='block min-w-0 space-y-2'>
      <span className='text-sm font-medium'>{t('Tag')}</span>
      <Input
        className='min-h-11'
        value={draft.tag ?? ''}
        disabled={disabled}
        onChange={(event) =>
          onChange(
            stringifyNativeDocument({ ...draft, tag: event.target.value }, 2)
          )
        }
      />
    </label>
  )
}

const catalogPageSize = 50

function ChannelModelCatalog({
  disabled,
  fetchModels,
  fetchCatalogOnOpen,
  savedModels,
  routes,
  groups,
  draft,
  onApply,
  onMap,
  onRemove,
  onCreate,
}: {
  disabled: boolean
  fetchModels?: () => Promise<string[]>
  fetchCatalogOnOpen?: boolean
  savedModels?: readonly string[]
  routes: NativeConfiguration['routing']['routes']
  groups: string[]
  draft: NativeChannelDraft | null
  onApply: (routes: NativeConfiguration['routing']['routes']) => void
  onMap: (
    route: NativeConfiguration['routing']['routes'][number],
    model: string
  ) => void
  onRemove: (attachments: NativeChannelDraft['attachments']) => void
  onCreate?: (model: string, group: string) => void
}) {
  const { t } = useTranslation()
  const [search, setSearch] = useState('')
  const [page, setPage] = useState(1)
  const [mappingModel, setMappingModel] = useState('')
  const [mappingRouteId, setMappingRouteId] = useState('')
  const mappingId = useId()
  const catalogSection = useRef<HTMLElement>(null)
  const choices = useRef<HTMLDetailsElement>(null)
  const fetchedOnOpen = useRef(false)
  const catalog = useMutation({ mutationFn: fetchModels })
  const { mutate } = catalog
  useEffect(() => {
    if (!fetchCatalogOnOpen || !fetchModels || fetchedOnOpen.current) return
    fetchedOnOpen.current = true
    mutate()
    catalogSection.current?.scrollIntoView?.({ block: 'nearest' })
  }, [fetchCatalogOnOpen, fetchModels, mutate])
  const availableModels = catalog.data ?? savedModels
  const models = useMemo(() => new Set(availableModels), [availableModels])
  const attached = useMemo(
    () =>
      new Map(
        draft?.attachments.map((item) => [item.route_id, item.upstream_model])
      ),
    [draft?.attachments]
  )
  const routesByModel = useMemo(() => {
    const index = new Map<string, typeof routes>()
    for (const route of routes) {
      const model = attached.get(route.id) ?? route.model
      const matches = index.get(model)
      if (matches) matches.push(route)
      else index.set(model, [route])
    }
    return index
  }, [attached, routes])
  const mappingRoute = routes.find(
    (route) => String(route.id) === mappingRouteId
  )
  const canMap =
    mappingRoute &&
    models.has(mappingModel) &&
    routes.some(
      (route) =>
        route.model === mappingRoute.model &&
        attached.get(route.id) !== mappingModel
    )
  const missingAttachments =
    catalog.isSuccess && models.size > 0
      ? (draft?.attachments.filter(
          (item) => !models.has(item.upstream_model)
        ) ?? [])
      : []
  const missing = new Map<string, NativeChannelDraft['attachments']>()
  for (const item of missingAttachments) {
    const matches = missing.get(item.upstream_model)
    if (matches) matches.push(item)
    else missing.set(item.upstream_model, [item])
  }
  const configuredModels = new Set(attached.values())
  const removingLastModel =
    draft?.enabled === true &&
    missingAttachments.length === draft.attachments.length
  const createGroup = groups[0] ?? ''
  const available = [...new Set([...models, ...routesByModel.keys()])].filter(
    (model) => {
      const matches = routesByModel.get(model) ?? []
      return matches.length
        ? matches.some((route) => !attached.has(route.id))
        : !configuredModels.has(model)
    }
  )
  const query = search.trim().toLowerCase()
  const filtered = available.filter((model) =>
    model.toLowerCase().includes(query)
  )
  const matchingRoutes = filtered.flatMap((model) =>
    (routesByModel.get(model) ?? []).filter((route) => !attached.has(route.id))
  )
  const pageCount = Math.max(1, Math.ceil(filtered.length / catalogPageSize))
  const currentPage = Math.min(page, pageCount)
  const visibleModels = filtered.slice(
    (currentPage - 1) * catalogPageSize,
    currentPage * catalogPageSize
  )

  function addBindings(bindings: typeof routes) {
    if (choices.current) choices.current.open = false
    onApply(bindings)
  }

  return (
    <section ref={catalogSection} className='min-w-0 space-y-3'>
      {fetchModels && (
        <Button
          type='button'
          variant='outline'
          className='min-h-11'
          disabled={disabled || catalog.isPending}
          onClick={() => {
            setPage(1)
            setMappingModel('')
            setMappingRouteId('')
            catalog.mutate()
          }}
        >
          {catalog.isPending ? t('Loading') : t('Fetch Models')}
        </Button>
      )}
      {availableModels && (
        <p role='status' className='text-muted-foreground text-xs'>
          {t('Upstream catalog')} · {models.size}
        </p>
      )}
      {catalog.isError && (
        <p role='alert' className='text-destructive text-sm'>
          {getServerErrorMessage(catalog.error) ?? t('Failed to load data')}
        </p>
      )}
      {missing.size > 0 && (
        <details className='min-w-0 border-t'>
          <summary className='min-h-11 cursor-pointer py-3 text-sm font-medium'>
            {t('Models missing from the upstream catalog')} ({missing.size})
          </summary>
          <ul className='max-h-52 overflow-y-auto'>
            {[...missing].map(([model, attachments]) => {
              const labelId = `${mappingId}-missing-${attachments[0].route_id}`
              return (
                <li
                  key={model}
                  className='flex min-w-0 flex-wrap items-center justify-between gap-2 py-1'
                >
                  <span
                    id={labelId}
                    className='min-w-0 text-sm [overflow-wrap:anywhere]'
                  >
                    {model}
                  </span>
                  <Button
                    type='button'
                    variant='outline'
                    size='sm'
                    className='min-h-11'
                    aria-describedby={labelId}
                    disabled={
                      disabled ||
                      (draft?.enabled === true &&
                        draft.attachments.length === attachments.length)
                    }
                    onClick={() => onRemove(attachments)}
                  >
                    {t('Remove')}
                  </Button>
                </li>
              )
            })}
          </ul>
          {removingLastModel && (
            <p className='text-muted-foreground text-xs'>
              {t(
                'Disable the channel before removing its last configured model.'
              )}
            </p>
          )}
          {missing.size > 1 && (
            <Button
              type='button'
              variant='outline'
              className='my-2 min-h-11'
              disabled={disabled || removingLastModel}
              onClick={() => onRemove(missingAttachments)}
            >
              {t('Remove from draft')} ({missing.size})
            </Button>
          )}
        </details>
      )}
      <details ref={choices} className='group/catalog min-w-0 border-t'>
        <summary className='flex min-h-12 cursor-pointer list-none items-center justify-between gap-3 py-2 text-sm font-medium [&::-webkit-details-marker]:hidden'>
          <span>
            {t('Available models')}{' '}
            <span className='text-muted-foreground tabular-nums'>
              ({available.length})
            </span>
          </span>
          <ChevronDown
            aria-hidden='true'
            className='size-4 shrink-0 group-open/catalog:rotate-180'
          />
        </summary>
        <section
          aria-label={t('Available models')}
          className='min-w-0 space-y-3 pb-3'
        >
          <Input
            className='min-h-11'
            aria-label={t('Search models')}
            placeholder={t('Search models')}
            value={search}
            onChange={(event) => {
              setSearch(event.target.value)
              setPage(1)
            }}
          />
          {onCreate &&
            !createGroup &&
            filtered.some((model) => !routesByModel.has(model)) && (
              <p className='text-muted-foreground text-xs'>
                {t('Group is required')}
              </p>
            )}
          {matchingRoutes.length > 0 && (
            <Button
              type='button'
              className='min-h-11 max-w-full whitespace-normal'
              variant='outline'
              disabled={disabled}
              onClick={() => addBindings(matchingRoutes)}
            >
              {t('Add matching bindings ({{count}})', {
                count: matchingRoutes.length,
              })}
            </Button>
          )}
          {draft && models.size > 0 && routes.length > 0 && (
            <details className='border-border rounded-md border p-3'>
              <summary className='min-h-8 cursor-pointer text-sm font-medium'>
                {t('Model Mapping')}
              </summary>
              <fieldset
                disabled={disabled || catalog.isPending}
                className='space-y-2'
              >
                <label htmlFor={`${mappingId}-upstream`} className='text-sm'>
                  {t('Upstream model')}
                </label>
                <Combobox
                  id={`${mappingId}-upstream`}
                  aria-label={t('Upstream model')}
                  placeholder={t('Search models')}
                  emptyText={t('No models found')}
                  maxVisibleOptions={catalogPageSize}
                  options={[...models].map((model) => ({
                    value: model,
                    label: model,
                  }))}
                  value={mappingModel}
                  onValueChange={(model) => setMappingModel(model ?? '')}
                />
                <label htmlFor={`${mappingId}-route`} className='text-sm'>
                  {t('Model name')}
                </label>
                <Combobox
                  id={`${mappingId}-route`}
                  aria-label={t('Model name')}
                  placeholder={t('Search models')}
                  emptyText={t('No models found')}
                  maxVisibleOptions={catalogPageSize}
                  options={[
                    ...new Map(
                      routes.map((route) => [route.model, route])
                    ).values(),
                  ].map((route) => ({
                    value: String(route.id),
                    label: route.model,
                  }))}
                  value={mappingRouteId}
                  onValueChange={(id) => setMappingRouteId(id ?? '')}
                />
                <Button
                  type='button'
                  variant='outline'
                  disabled={!canMap || disabled}
                  onClick={() => {
                    if (!canMap) return
                    onMap(mappingRoute, mappingModel)
                    setMappingRouteId('')
                    setMappingModel('')
                  }}
                >
                  {t('Confirm')}
                </Button>
              </fieldset>
            </details>
          )}
          {filtered.length > 0 ? (
            <ul
              key={currentPage}
              className='bg-muted/40 max-h-64 overflow-y-auto rounded-md p-3'
            >
              {visibleModels.map((model) => {
                const matches = (routesByModel.get(model) ?? []).filter(
                  (route) => !attached.has(route.id)
                )
                return (
                  <li
                    key={model}
                    className='flex min-w-0 flex-wrap items-center justify-between gap-x-3 gap-y-1 py-1.5'
                  >
                    <span className='min-w-0 font-mono text-sm [overflow-wrap:anywhere]'>
                      {model}
                    </span>
                    {matches.length > 0 && (
                      <Button
                        type='button'
                        variant='outline'
                        size='sm'
                        className='min-h-11'
                        disabled={disabled}
                        onClick={() => addBindings(matches)}
                      >
                        {t('Add')}
                      </Button>
                    )}
                    {onCreate && matches.length === 0 && (
                      <Button
                        type='button'
                        variant='ghost'
                        disabled={disabled || !createGroup}
                        onClick={() => onCreate(model, createGroup)}
                      >
                        {t('Add Model')}
                      </Button>
                    )}
                  </li>
                )
              })}
            </ul>
          ) : (
            <p className='text-muted-foreground text-sm'>
              {t('No models found')}
            </p>
          )}
          {pageCount > 1 && (
            <div className='flex flex-wrap items-center justify-between gap-2'>
              <p role='status' className='text-muted-foreground text-sm'>
                {t('Page {{current}} of {{total}}', {
                  current: currentPage,
                  total: pageCount,
                })}
              </p>
              <div className='flex flex-wrap gap-2'>
                <Button
                  type='button'
                  variant='outline'
                  size='sm'
                  className='min-h-11'
                  disabled={currentPage === 1}
                  onClick={() => setPage(currentPage - 1)}
                >
                  {t('Previous page')}
                </Button>
                <Button
                  type='button'
                  variant='outline'
                  size='sm'
                  className='min-h-11'
                  disabled={currentPage === pageCount}
                  onClick={() => setPage(currentPage + 1)}
                >
                  {t('Next page')}
                </Button>
              </div>
            </div>
          )}
        </section>
      </details>
    </section>
  )
}

/** Common edits share the exact draft and CAS version with the advanced editor. */
export function NativeChannelFields({
  document,
  configuration,
  disabled,
  credential,
  fetchCatalogOnOpen,
  fetchModels,
  onCreateModel,
  onChange,
}: {
  document: string
  configuration: NativeConfiguration
  disabled: boolean
  credential?: ReactNode
  fetchCatalogOnOpen?: boolean
  fetchModels?: () => Promise<string[]>
  onCreateModel?: (model: string, group: string) => void
  onChange: (document: string) => void
}) {
  const { t } = useTranslation()
  const modelFields = useRef<HTMLDetailsElement>(null)
  function editModelBindings() {
    if (!modelFields.current) return
    modelFields.current.open = true
    modelFields.current.scrollIntoView?.({ block: 'start' })
    modelFields.current.querySelector('summary')?.focus({ preventScroll: true })
  }
  let draft: Record<string, unknown>
  try {
    draft = parseNativeDocument(document)
  } catch {
    // Keep invalid advanced input intact so the operator can correct it.
    return null
  }
  function change(key: string, value: string | boolean) {
    const updates: Record<string, string | boolean> = { [key]: value }
    if (key === 'protocol' && typeof value === 'string') {
      const endpoint = presetProtocolEndpoint(
        provider,
        draft.protocol,
        draft.endpoint,
        value
      )
      if (endpoint) updates.endpoint = endpoint
    }
    const next = { ...draft, ...updates }
    if (
      (key === 'endpoint' || key === 'protocol') &&
      Array.isArray(draft.contexts)
    ) {
      next.contexts = draft.contexts.map((context) =>
        updateNativeContextDestination(context, updates)
      )
    }
    onChange(stringifyNativeDocument(next, 2))
  }
  const savedChannel = configuration.routing.upstreams.find(
    (channel) => channel.id === draft.id
  )
  const catalogMatchesDraft =
    savedChannel &&
    savedChannel.endpoint === draft.endpoint &&
    savedChannel.protocol === draft.protocol
  const savedModels =
    savedChannel && configuration.bindings.discoveries[savedChannel.id]?.models
  const presets = configuration.provider_presets ?? []
  const provider = resolveNativeProvider(
    configuration,
    draft.provider,
    draft.protocol,
    draft.endpoint
  )
  const attachmentModels = Array.isArray(draft.attachments)
    ? draft.attachments
        .map((target) => target?.upstream_model)
        .filter(
          (model): model is string => typeof model === 'string' && !!model
        )
    : []
  const detection = detectUpstreamApi(configuration, {
    endpoint: draft.endpoint,
    provider: draft.provider,
    id: draft.id,
    models: attachmentModels,
  })
  function applyDetection() {
    if (!('protocol' in detection)) return
    const fields = {
      protocol: detection.protocol,
      endpoint: detection.endpoint,
    }
    const contexts = Array.isArray(draft.contexts)
      ? draft.contexts.map((context) =>
          updateNativeContextDestination(context, fields)
        )
      : draft.contexts
    onChange(stringifyNativeDocument({ ...draft, ...fields, contexts }, 2))
  }
  function selectProvider(id: string | null) {
    const preset = presets.find((item) => item.id === id)
    if (!preset) return
    const next: Record<string, unknown> = { ...draft, provider: preset.id }
    if (
      !configuration.routing.upstreams.some((item) => item.id === draft.id) &&
      preset.id !== 'custom'
    ) {
      const detected = detectUpstreamApi(configuration, {
        endpoint: preset.endpoint,
        provider: preset.id,
        models: [],
      })
      const defaults = {
        protocol: detected.source === 'ambiguous' ? '' : preset.protocol,
        endpoint: preset.endpoint || draft.endpoint,
      }
      Object.assign(next, defaults)
      if (Array.isArray(draft.contexts)) {
        next.contexts = draft.contexts.map((context) =>
          updateNativeContextDestination(context, defaults)
        )
      }
    }
    onChange(stringifyNativeDocument(next, 2))
  }
  const enabledField = (
    <label className='flex min-h-11 items-center gap-3 text-sm @min-[40rem]/routing:col-span-2'>
      <Checkbox
        checked={draft.enabled === true}
        disabled={disabled}
        onCheckedChange={(checked) => change('enabled', checked === true)}
      />
      {t('Enabled')}
    </label>
  )
  const channel = readChannelDraft(document)
  const groups = channel?.groups ?? []
  const routes = configuration.routing.routes.filter((route) =>
    groups.includes(route.group)
  )
  const changeRouting = (
    field: 'priority' | 'weight',
    input: HTMLInputElement
  ) => {
    if (input.value !== '' && !input.validity.valid) return
    if (input.value === '') {
      onChange(stringifyNativeDocument({ ...draft, [field]: '' }, 2))
      return
    }
    onChange(
      stringifyNativeDocument({ ...draft, [field]: input.valueAsNumber }, 2)
    )
  }
  const routingFields = (
    <fieldset className='min-w-0 space-y-3' disabled={disabled}>
      <legend className='text-sm font-medium'>{t('Groups *')}</legend>
      <div className='flex flex-wrap gap-x-5 gap-y-2'>
        {configuration.routing.groups.map(({ name }) => (
          <div key={name} className='flex min-h-9 items-center gap-2 text-sm'>
            <Checkbox
              aria-label={name}
              checked={groups.includes(name)}
              onCheckedChange={(checked) =>
                onChange(
                  stringifyNativeDocument(
                    {
                      ...draft,
                      groups: checked
                        ? [...groups, name]
                        : groups.filter((group) => group !== name),
                    },
                    2
                  )
                )
              }
            />
            <span>{name}</span>
          </div>
        ))}
      </div>
      <div className='grid min-w-0 grid-cols-2 gap-4'>
        {(
          [
            ['priority', t('Priority'), 65_535],
            ['weight', t('Weight'), 4_294_967_295],
          ] as const
        ).map(([field, label, max]) => (
          <label key={field} className='block min-w-0 space-y-2 text-sm'>
            <span>{label}</span>
            <Input
              className='min-h-11 tabular-nums'
              type='number'
              min={0}
              max={max}
              step={1}
              placeholder={t('Mixed values')}
              required={channel?.[field] === ''}
              value={
                channel?.[field] ??
                commonChannelTargetValue(channel?.attachments ?? [], field) ??
                ''
              }
              onChange={(event) => changeRouting(field, event.currentTarget)}
            />
          </label>
        ))}
      </div>
    </fieldset>
  )
  const connectionFields = (
    <>
      <label className='block min-w-0 space-y-2'>
        <span className='text-sm font-medium'>{t('Upstream API')}</span>
        <select
          aria-label={t('Upstream API')}
          value={typeof draft.protocol === 'string' ? draft.protocol : ''}
          required
          disabled={disabled}
          className='border-input bg-background block min-h-11 w-full rounded-md border px-3 text-sm'
          onChange={(event) => change('protocol', event.target.value)}
        >
          <option value='' disabled>
            {t('Select the upstream API')}
          </option>
          {nativeProtocols.map((protocol) => (
            <option key={protocol} value={protocol}>
              {nativeProtocolLabel(protocol, t)}
            </option>
          ))}
        </select>
      </label>
      <label className='block min-w-0 space-y-2'>
        <span className='text-sm font-medium'>{t('Base URL')}</span>
        <Input
          type='url'
          className='min-h-11'
          value={typeof draft.endpoint === 'string' ? draft.endpoint : ''}
          required
          disabled={disabled}
          onChange={(event) => change('endpoint', event.target.value)}
        />
      </label>
      <div
        className='text-muted-foreground space-y-1 text-xs @min-[40rem]/routing:col-span-2'
        aria-live='polite'
      >
        {'protocol' in detection ? (
          <>
            <p>
              {detection.source === 'operation'
                ? t('API identified from the operation URL: {{api}}.', {
                    api: nativeProtocolLabel(detection.protocol, t),
                  })
                : detection.source === 'model'
                  ? t(
                      'Published targets for these models use {{api}} on this endpoint.',
                      { api: nativeProtocolLabel(detection.protocol, t) }
                    )
                  : t('This endpoint’s supplier preset suggests {{api}}.', {
                      api: nativeProtocolLabel(detection.protocol, t),
                    })}
            </p>
            {(detection.protocol !== draft.protocol ||
              detection.endpoint !== draft.endpoint) && (
              <Button
                type='button'
                size='sm'
                variant='outline'
                disabled={disabled}
                onClick={applyDetection}
              >
                {t('Use detected API and base URL')}
              </Button>
            )}
          </>
        ) : (
          <p>
            {detection.source === 'ambiguous'
              ? t(
                  'This endpoint can serve multiple APIs. Choose the API for the models bound to this channel.'
                )
              : t(
                  'The endpoint does not identify an API. Select it from the supplier’s API documentation.'
                )}
          </p>
        )}
      </div>
      {credential}
      {presets.length > 0 && (
        <>
          <div className='space-y-2'>
            <span className='text-sm font-medium'>
              {t('Upstream supplier')}
            </span>
            <Select
              value={provider?.id ?? 'custom'}
              disabled={disabled}
              onValueChange={selectProvider}
            >
              <SelectTrigger
                aria-label={t('Upstream supplier')}
                className='min-h-11 w-full'
              >
                <SelectValue>
                  <NativeProviderLogo icon={provider?.icon} />
                  {provider?.id === 'custom' ? t('Custom') : provider?.label}
                </SelectValue>
              </SelectTrigger>
              <SelectContent alignItemWithTrigger={false}>
                {presets.map((preset) => (
                  <SelectItem
                    key={preset.id}
                    value={preset.id}
                    label={preset.label}
                  >
                    <span className='flex items-center gap-2'>
                      <NativeProviderLogo icon={preset.icon} />
                      {preset.id === 'custom' ? t('Custom') : preset.label}
                    </span>
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <label className='block min-w-0 space-y-2'>
            <span className='text-sm font-medium'>{t('Internal notes')}</span>
            <Input
              maxLength={4096}
              disabled={disabled}
              value={typeof draft.note === 'string' ? draft.note : ''}
              onChange={(event) => change('note', event.target.value)}
            />
          </label>
        </>
      )}
    </>
  )
  return (
    <div className='min-w-0 space-y-4'>
      <div className='grid min-w-0 gap-x-5 gap-y-4 @min-[40rem]/routing:grid-cols-2'>
        <label className='block min-w-0 space-y-2'>
          <span className='text-sm font-medium'>{t('Name')}</span>
          <Input
            className='min-h-11'
            value={typeof draft.name === 'string' ? draft.name : ''}
            required
            disabled={disabled}
            onChange={(event) => change('name', event.target.value)}
          />
        </label>
        {connectionFields}
        {enabledField}
      </div>
      {routingFields}
      <section className='min-w-0 space-y-2' aria-label={t('Models')}>
        <h4 className='text-sm font-semibold'>{t('Models')}</h4>
        {!catalogMatchesDraft && (
          <p className='text-muted-foreground text-sm'>
            {t('Save the channel before fetching its upstream models.')}
          </p>
        )}
        {savedChannel && (
          <ChannelModelCatalog
            key={JSON.stringify([draft.id, draft.protocol, draft.endpoint])}
            disabled={disabled}
            fetchModels={catalogMatchesDraft ? fetchModels : undefined}
            fetchCatalogOnOpen={!!catalogMatchesDraft && fetchCatalogOnOpen}
            savedModels={catalogMatchesDraft ? savedModels : undefined}
            routes={routes}
            groups={groups}
            draft={readChannelDraft(document)}
            onCreate={onCreateModel}
            onMap={(route, model) => {
              const current = readChannelDraft(document)
              if (!current) return
              const matches = routes.filter(
                (item) => item.model === route.model
              )
              const next = matches.reduce(
                (next, item) =>
                  updateChannelAttachment(
                    toggleChannelAttachment(next, item, true, configuration),
                    item.id,
                    { upstream_model: model },
                    configuration.bindings.contexts
                  ),
                current
              )
              onChange(stringifyNativeDocument(next, 2))
              editModelBindings()
            }}
            onRemove={(attachments) => {
              const current = readChannelDraft(document)
              if (!current) return
              onChange(
                stringifyNativeDocument(
                  attachments.reduce(
                    (next, item) =>
                      toggleChannelAttachment(
                        next,
                        {
                          id: item.route_id,
                          model: item.upstream_model,
                          speeds: item.speeds,
                        },
                        false
                      ),
                    current
                  ),
                  2
                )
              )
              editModelBindings()
            }}
            onApply={(routes) => {
              const current = readChannelDraft(document)
              if (!current) return
              onChange(
                stringifyNativeDocument(
                  routes.reduce(
                    (next, route) =>
                      toggleChannelAttachment(next, route, true, configuration),
                    current
                  ),
                  2
                )
              )
              editModelBindings()
            }}
          />
        )}
        <NativeChannelModelsFields
          ref={modelFields}
          configuredOnly={!!savedChannel}
          document={document}
          configuration={configuration}
          disabled={disabled}
          onChange={onChange}
        />
      </section>
    </div>
  )
}
