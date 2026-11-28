/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { ChevronDown } from 'lucide-react'
import { useState, type Ref } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Input } from '@/components/ui/input'
import { Switch } from '@/components/ui/switch'

import {
  nativeProtocolLabel,
  nativeProtocols,
  newNativeContext,
  readChannelDraft,
  toggleChannelAttachment,
  updateChannelAttachment,
  type NativeChannelDraft,
  type NativeTargetFieldsValue,
} from '../lib/native-routing-form'
import type { NativeConfiguration } from '../native-api'
import { NativeTargetTimeoutField } from './native-target-timeout-field'

const fieldClass = 'block min-w-0 space-y-2 text-sm'
const passthroughFields = [
  'service_tier',
  'inference_geo',
  'speed',
  'safety_identifier',
  'include_obfuscation',
] as const

function IntegerField({
  label,
  value,
  disabled,
  onChange,
  min = 0,
  max,
}: {
  label: string
  value: number | string
  disabled: boolean
  onChange: (value: number | string) => void
  min?: number
  max: number
}) {
  return (
    <label className={fieldClass}>
      <span>{label}</span>
      <Input
        type='number'
        min={min}
        max={max}
        step={1}
        required
        className='min-h-11 tabular-nums'
        value={value}
        disabled={disabled}
        onChange={(event) => {
          const raw = event.target.value
          const number = Number(raw)
          onChange(raw !== '' && Number.isSafeInteger(number) ? number : raw)
        }}
      />
    </label>
  )
}

/** Model bindings edit the same channel draft that the CAS save publishes. */
export function NativeChannelModelsFields({
  ref,
  configuredOnly = false,
  document,
  configuration,
  disabled,
  onChange,
}: {
  ref?: Ref<HTMLDetailsElement>
  configuredOnly?: boolean
  document: string
  configuration: NativeConfiguration
  disabled: boolean
  onChange: (document: string) => void
}) {
  const { t } = useTranslation()
  const [modelSearch, setModelSearch] = useState('')
  const [selected, setSelected] = useState<string[]>([])
  const [collapsed, setCollapsed] = useState<number[]>(() => {
    const initial = readChannelDraft(document)
    return (
      initial?.attachments
        .filter((attachment) =>
          initial.contexts.some(
            (context) =>
              context.upstream_model === attachment.upstream_model &&
              context.contract_revision.trim() !== ''
          )
        )
        .map((attachment) => attachment.route_id) ?? []
    )
  })
  const draft = readChannelDraft(document)
  if (!draft) return null
  const current = draft
  const routes = new Map(
    configuration.routing.routes.map((route) => [route.id, route])
  )
  const configured = new Map<string, NativeChannelDraft['attachments']>()
  for (const item of draft.attachments) {
    const name = routes.get(item.route_id)?.model ?? item.upstream_model
    const key = JSON.stringify([name, item.upstream_model])
    const group = configured.get(key)
    if (group) group.push(item)
    else configured.set(key, [item])
  }
  const removingLastModel = draft.enabled && configured.size === 1
  const openAi =
    draft.protocol === 'openai_chat' || draft.protocol === 'openai_responses'
  const put = (next: NativeChannelDraft) =>
    onChange(JSON.stringify(next, null, 2))
  const available = configuration.routing.routes.filter(
    (route) =>
      (draft.groups ?? []).includes(route.group) &&
      !draft.attachments.some((item) => item.route_id === route.id)
  )
  const names = [...new Set(available.map((route) => route.model))]
  const searchTerm = modelSearch.trim().toLowerCase()
  const matches = names.filter((model) =>
    model.toLowerCase().includes(searchTerm)
  )
  const selectedRoutes = available.filter((route) =>
    selected.includes(route.model)
  )
  const selectedCount = new Set(selectedRoutes.map((route) => route.model)).size
  const allMatchesSelected =
    matches.length > 0 && matches.every((model) => selected.includes(model))
  const someMatchesSelected = matches.some((model) => selected.includes(model))
  function addSelectedModels() {
    if (selectedRoutes.length === 0) return
    put(
      selectedRoutes.reduce(
        (next, route) =>
          toggleChannelAttachment(next, route, true, configuration),
        current
      )
    )
    setCollapsed((ids) =>
      ids.filter((id) => !selectedRoutes.some((route) => route.id === id))
    )
    setSelected([])
  }
  const tool = ['deepseek_search', 'deepseek_fetch'].includes(draft.protocol)
  const inputOnly = ['openai_embeddings', 'jina_rerank'].includes(
    draft.protocol
  )
  function patchContext(
    model: string,
    patch: Partial<
      Pick<
        NativeChannelDraft['contexts'][number],
        | 'max_input_tokens'
        | 'max_output_tokens'
        | 'contract_revision'
        | 'enforces_output_token_limit'
      >
    >
  ) {
    const contexts = current.contexts.some(
      (context) => context.upstream_model === model
    )
      ? current.contexts
      : [...current.contexts, newNativeContext(current, model)]
    put({
      ...current,
      contexts: contexts.map((context) =>
        context.upstream_model === model ? { ...context, ...patch } : context
      ),
    })
  }
  return (
    <details
      ref={ref}
      className='group/models min-w-0 space-y-3 border-t'
      aria-label={t('Models')}
    >
      <summary className='flex min-h-12 cursor-pointer list-none items-center gap-3 py-2 focus-visible:outline-2 [&::-webkit-details-marker]:hidden'>
        <span className='text-sm font-medium'>
          {configuredOnly ? t('Configured') : t('Models')}
        </span>
        <span className='text-muted-foreground min-w-0 flex-1 truncate text-sm'>
          {[...configured.values()]
            .slice(0, 2)
            .map(
              ([attachment]) =>
                configuration.routing.routes.find(
                  (route) => route.id === attachment.route_id
                )?.model ?? attachment.upstream_model
            )
            .join(' · ') || t('No models selected')}
        </span>
        {configured.size > 0 && (
          <span className='text-muted-foreground shrink-0 text-xs tabular-nums'>
            {t('{{count}} selected', { count: configured.size })}
          </span>
        )}
        <ChevronDown
          aria-hidden='true'
          className='size-4 shrink-0 transition-transform group-open/models:rotate-180 motion-reduce:transition-none'
        />
      </summary>
      {[...configured.values()].map((attachments) => {
        const [attachment] = attachments
        const route = routes.get(attachment.route_id)
        function patchModel(patch: Partial<NativeTargetFieldsValue>) {
          put(
            attachments.reduce(
              (next, item) =>
                updateChannelAttachment(
                  next,
                  item.route_id,
                  patch,
                  configuration.bindings.contexts
                ),
              current
            )
          )
        }
        const enabledProtocols = attachment.client_protocols ?? []
        const policy = attachment.payload
        const flags =
          typeof policy === 'object' &&
          policy.legacy_openai &&
          typeof policy.legacy_openai === 'object'
            ? (policy.legacy_openai as Record<string, unknown>)
            : undefined
        const policyValue = flags
          ? 'legacy_openai'
          : typeof policy === 'string'
            ? policy
            : ''
        const policies = [
          'adapt',
          'passthrough',
          ...(openAi || flags ? ['legacy_openai'] : []),
        ]
        const metadata = configuration.bindings.models[attachment.route_id]
        const context = draft.contexts.find(
          (item) => item.upstream_model === attachment.upstream_model
        )
        const enforcesOutputLimit = context?.enforces_output_token_limit
        const publishedContract = configuration.bindings.contexts.some(
          (item) =>
            item.upstream_id === draft.id &&
            item.upstream_model === attachment.upstream_model &&
            item.contract_revision !== ''
        )
        return (
          <details
            key={attachment.route_id}
            className='group/model min-w-0 border-t'
            open={!collapsed.includes(attachment.route_id)}
            onToggle={(event) => {
              const open = event.currentTarget.open
              setCollapsed((ids) =>
                open
                  ? ids.includes(attachment.route_id)
                    ? ids.filter((id) => id !== attachment.route_id)
                    : ids
                  : ids.includes(attachment.route_id)
                    ? ids
                    : [...ids, attachment.route_id]
              )
            }}
          >
            <summary className='flex min-h-16 min-w-0 cursor-pointer list-none items-center justify-between gap-3 py-4 focus-visible:outline-2 focus-visible:outline-offset-4 [&::-webkit-details-marker]:hidden'>
              <span className='min-w-0 space-y-1'>
                <span className='block font-mono text-sm font-medium [overflow-wrap:anywhere]'>
                  {metadata?.display_name ||
                    route?.model ||
                    attachment.upstream_model}
                </span>
              </span>
              <ChevronDown
                className='text-muted-foreground size-4 shrink-0 transition-transform group-open/model:rotate-180 motion-reduce:transition-none'
                aria-hidden='true'
              />
            </summary>
            <fieldset
              className='min-w-0 space-y-4 pb-5'
              data-route-id={attachment.route_id}
            >
              <legend className='sr-only'>
                {route?.model ?? attachment.upstream_model}
              </legend>
              <div className='flex flex-wrap items-center justify-end gap-2'>
                {removingLastModel && (
                  <p className='text-muted-foreground text-xs'>
                    {t(
                      'Disable the channel before removing its last configured model.'
                    )}
                  </p>
                )}
                <Button
                  type='button'
                  variant='ghost'
                  className='min-h-11'
                  disabled={disabled || removingLastModel}
                  onClick={() =>
                    put(
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
                      )
                    )
                  }
                >
                  {t('Remove')}
                </Button>
              </div>
              <label className={fieldClass}>
                <span>{t('Upstream model')}</span>
                <Input
                  className='min-h-11 font-mono'
                  value={attachment.upstream_model}
                  maxLength={200}
                  required
                  disabled={disabled}
                  onChange={(event) =>
                    patchModel({ upstream_model: event.target.value })
                  }
                />
              </label>
              <NativeTargetTimeoutField
                value={attachment.timeout_ms}
                configuration={configuration}
                enabled={draft.enabled}
                disabled={disabled}
                onChange={(timeout_ms) => patchModel({ timeout_ms })}
              />
              <details open={!publishedContract}>
                <summary className='cursor-pointer py-2 text-sm font-medium'>
                  {t('Supplier contract')}
                </summary>
                <p className='text-muted-foreground mb-4 text-sm'>
                  {t(
                    'Use the token limits and revision documented by this supplier for this model.'
                  )}
                </p>
                <div className='grid min-w-0 gap-4 @min-[40rem]/routing:grid-cols-2'>
                  <IntegerField
                    label={t('Maximum input tokens')}
                    value={context?.max_input_tokens ?? ''}
                    min={tool ? 0 : 1}
                    max={tool ? 0 : 4_294_967_295}
                    disabled={disabled}
                    onChange={(value) =>
                      patchContext(attachment.upstream_model, {
                        max_input_tokens: value,
                      })
                    }
                  />
                  <IntegerField
                    label={t('Maximum output tokens')}
                    value={context?.max_output_tokens ?? ''}
                    min={tool || inputOnly ? 0 : 1}
                    max={tool || inputOnly ? 0 : 4_294_967_295}
                    disabled={disabled}
                    onChange={(value) =>
                      patchContext(attachment.upstream_model, {
                        max_output_tokens: value,
                      })
                    }
                  />
                  {draft.protocol === 'anthropic_messages' && (
                    <label
                      className={`${fieldClass} @min-[40rem]/routing:col-span-2`}
                    >
                      <span>{t('Requested output token limit')}</span>
                      <select
                        className='bg-muted/65 min-h-11 w-full rounded-lg px-3 text-sm'
                        value={
                          enforcesOutputLimit === undefined
                            ? ''
                            : String(enforcesOutputLimit)
                        }
                        disabled={disabled}
                        onChange={(event) => {
                          const value = event.target.value
                          patchContext(attachment.upstream_model, {
                            enforces_output_token_limit:
                              value === '' ? undefined : value === 'true',
                          })
                        }}
                      >
                        <option value=''>{t('Unverified')}</option>
                        <option value='true'>
                          {t('Verified by operator')}
                        </option>
                        <option value='false'>{t('Ignores the limit')}</option>
                      </select>
                      <span className='text-muted-foreground block'>
                        {t(
                          'Routing skips this supplier model when it ignores the requested output limit.'
                        )}
                      </span>
                    </label>
                  )}
                  <label
                    className={`${fieldClass} @min-[40rem]/routing:col-span-2`}
                  >
                    <span>{t('Contract revision')}</span>
                    <Input
                      className='min-h-11'
                      value={context?.contract_revision ?? ''}
                      maxLength={512}
                      required
                      disabled={disabled}
                      onChange={(event) =>
                        patchContext(attachment.upstream_model, {
                          contract_revision: event.target.value,
                        })
                      }
                    />
                  </label>
                </div>
              </details>
              <details>
                <summary className='cursor-pointer py-2 text-sm font-medium'>
                  {t('Request forwarding')}
                </summary>
                <label className={fieldClass}>
                  <span>{t('Request forwarding')}</span>
                  <select
                    className='bg-muted/65 min-h-11 w-full rounded-lg px-3 text-sm'
                    value={policyValue}
                    disabled={disabled}
                    onChange={(event) => {
                      const value = event.target.value
                      patchModel({
                        payload:
                          value === 'legacy_openai'
                            ? { legacy_openai: {} }
                            : value,
                      })
                    }}
                  >
                    {!policies.includes(policyValue) && (
                      <option value={policyValue}>
                        {policyValue || t('Custom')}
                      </option>
                    )}
                    <option value='adapt'>
                      {t('Adapt protocol and model')}
                    </option>
                    <option value='passthrough'>
                      {t('Forward the request body unchanged')}
                    </option>
                    {(openAi || flags) && (
                      <option value='legacy_openai'>
                        {t('Filter OpenAI request fields')}
                      </option>
                    )}
                  </select>
                </label>
                <fieldset className='mt-3 space-y-2' disabled={disabled}>
                  <legend className='text-muted-foreground text-sm'>
                    {t('Inbound protocol')}
                  </legend>
                  <Button
                    type='button'
                    size='sm'
                    variant={
                      attachment.client_protocols == null
                        ? 'secondary'
                        : 'outline'
                    }
                    aria-pressed={attachment.client_protocols == null}
                    disabled={disabled}
                    onClick={() =>
                      patchModel({
                        client_protocols: null,
                      })
                    }
                  >
                    {t('Automatic')}
                  </Button>
                  <div className='grid gap-2 sm:grid-cols-2'>
                    {nativeProtocols.map((protocol) => {
                      const enabled = enabledProtocols.includes(protocol)
                      return (
                        <label
                          key={protocol}
                          className='flex items-center gap-2 text-sm'
                        >
                          <Checkbox
                            checked={enabled}
                            disabled={
                              disabled ||
                              (enabled && enabledProtocols.length === 1)
                            }
                            onCheckedChange={(checked) =>
                              patchModel({
                                client_protocols: checked
                                  ? [...enabledProtocols, protocol]
                                  : enabledProtocols.filter(
                                      (value) => value !== protocol
                                    ),
                              })
                            }
                          />
                          {nativeProtocolLabel(protocol, t)}
                        </label>
                      )
                    })}
                  </div>
                </fieldset>
                {flags && (
                  <fieldset className='mt-3 space-y-2'>
                    <legend className='text-muted-foreground text-sm'>
                      {t('Fields allowed upstream')}
                    </legend>
                    {passthroughFields.map((field) => {
                      const key = `allow_${field}`
                      return (
                        <label
                          key={field}
                          className='flex min-h-10 items-center justify-between gap-4 border-b text-sm'
                        >
                          <code>
                            {field === 'include_obfuscation'
                              ? 'include'
                              : field}
                          </code>
                          <Switch
                            checked={flags[key] === true}
                            disabled={disabled}
                            onCheckedChange={(checked) =>
                              patchModel({
                                payload: {
                                  legacy_openai: {
                                    ...flags,
                                    [key]: checked,
                                  },
                                },
                              })
                            }
                          />
                        </label>
                      )
                    })}
                  </fieldset>
                )}
              </details>
            </fieldset>
          </details>
        )
      })}
      {!configuredOnly && available.length > 0 && (
        <section
          className='bg-muted/40 min-w-0 space-y-4 rounded-xl p-4 sm:p-5'
          aria-label={t('Available models')}
        >
          <div className='space-y-1'>
            <h5 className='text-sm font-medium'>{t('Available models')}</h5>
          </div>
          <Input
            aria-label={t('Search models')}
            placeholder={t('Search models')}
            className='min-h-11'
            value={modelSearch}
            disabled={disabled}
            onChange={(event) => setModelSearch(event.target.value)}
          />
          <div className='flex flex-wrap items-center justify-between gap-3'>
            <label className='flex min-h-11 cursor-pointer items-center gap-3 text-sm'>
              <Checkbox
                checked={allMatchesSelected}
                indeterminate={someMatchesSelected && !allMatchesSelected}
                disabled={disabled || matches.length === 0}
                onCheckedChange={(checked) =>
                  setSelected((ids) =>
                    checked
                      ? [...new Set([...ids, ...matches])]
                      : ids.filter((model) => !matches.includes(model))
                  )
                }
              />
              {t('Select all matches')}
            </label>
            <span
              className='text-muted-foreground text-sm tabular-nums'
              role='status'
            >
              {t('{{count}} selected', { count: selectedCount })}
            </span>
          </div>
          <div className='max-h-56 min-w-0 overflow-y-auto overscroll-contain'>
            {matches.length === 0 ? (
              <p className='text-muted-foreground py-5 text-center text-sm'>
                {t('No results found')}
              </p>
            ) : (
              <div className='grid min-w-0 gap-1 @min-[40rem]/routing:grid-cols-2'>
                {matches.map((model) => (
                  <label
                    key={model}
                    className='hover:bg-muted/70 flex min-h-12 min-w-0 cursor-pointer items-start gap-3 rounded-lg px-3 py-2.5'
                  >
                    <Checkbox
                      className='mt-0.5 shrink-0'
                      checked={selected.includes(model)}
                      disabled={disabled}
                      onCheckedChange={(checked) =>
                        setSelected((ids) =>
                          checked
                            ? [...ids, model]
                            : ids.filter((id) => id !== model)
                        )
                      }
                    />
                    <span className='min-w-0 space-y-0.5'>
                      <span className='block font-mono text-sm [overflow-wrap:anywhere]'>
                        {model}
                      </span>
                    </span>
                  </label>
                ))}
              </div>
            )}
          </div>
          <div className='flex flex-wrap items-center gap-2'>
            <Button
              type='button'
              className='min-h-11'
              disabled={disabled || selectedRoutes.length === 0}
              onClick={addSelectedModels}
            >
              {t('Add selected models')}
            </Button>
            {selectedRoutes.length > 0 && (
              <Button
                type='button'
                variant='ghost'
                className='min-h-11'
                disabled={disabled}
                onClick={() => setSelected([])}
              >
                {t('Clear')}
              </Button>
            )}
          </div>
        </section>
      )}
      {!configuredOnly && configuration.routing.routes.length === 0 && (
        <p className='text-muted-foreground text-sm'>
          {t('No models available')}
        </p>
      )}
    </details>
  )
}
