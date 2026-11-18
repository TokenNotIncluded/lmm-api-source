/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { ChevronDown } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Input } from '@/components/ui/input'

import {
  newNativeContext,
  readChannelDraft,
  toggleChannelAttachment,
  updateChannelAttachment,
  type NativeChannelDraft,
} from '../lib/native-routing-form'
import type { NativeConfiguration } from '../native-api'
import { NativeTargetTimeoutField } from './native-target-timeout-field'

const fieldClass = 'block min-w-0 space-y-2 text-sm'

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
  document,
  configuration,
  disabled,
  onChange,
}: {
  document: string
  configuration: NativeConfiguration
  disabled: boolean
  onChange: (document: string) => void
}) {
  const { t } = useTranslation()
  const [modelSearch, setModelSearch] = useState('')
  const [selected, setSelected] = useState<number[]>([])
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
  const put = (next: NativeChannelDraft) =>
    onChange(JSON.stringify(next, null, 2))
  const available = configuration.routing.routes.filter(
    (route) => !draft.attachments.some((item) => item.route_id === route.id)
  )
  const searchTerm = modelSearch.trim().toLowerCase()
  const matches = available.filter(
    (route) =>
      route.model.toLowerCase().includes(searchTerm) ||
      route.group.toLowerCase().includes(searchTerm)
  )
  const selectedRoutes = available.filter((route) =>
    selected.includes(route.id)
  )
  const allMatchesSelected =
    matches.length > 0 && matches.every((route) => selected.includes(route.id))
  const someMatchesSelected = matches.some((route) =>
    selected.includes(route.id)
  )
  function addSelectedModels() {
    if (selectedRoutes.length === 0) return
    put(
      selectedRoutes.reduce(
        (next, route) =>
          toggleChannelAttachment(next, route, true, configuration),
        current
      )
    )
    setCollapsed((ids) => ids.filter((id) => !selected.includes(id)))
    setSelected([])
  }
  const tool = ['deepseek_search', 'deepseek_fetch'].includes(draft.protocol)
  const inputOnly = ['openai_embeddings', 'jina_rerank'].includes(
    draft.protocol
  )
  function patchContext(
    model: string,
    key: 'max_input_tokens' | 'max_output_tokens' | 'contract_revision',
    value: number | string
  ) {
    const contexts = current.contexts.some(
      (context) => context.upstream_model === model
    )
      ? current.contexts
      : [...current.contexts, newNativeContext(current, model)]
    put({
      ...current,
      contexts: contexts.map((context) =>
        context.upstream_model === model
          ? { ...context, [key]: value }
          : context
      ),
    })
  }
  return (
    <section className='min-w-0 space-y-5' aria-label={t('Models')}>
      <div className='space-y-2'>
        <h4 className='font-medium'>{t('Models')}</h4>
        <p className='text-muted-foreground text-sm'>
          {t(
            'Lower priority values run first. Weight distributes requests within the same priority.'
          )}
        </p>
      </div>
      {available.length > 0 && (
        <section
          className='bg-muted/40 min-w-0 space-y-4 rounded-xl p-4 sm:p-5'
          aria-label={t('Available models')}
        >
          <div className='space-y-1'>
            <h5 className='text-sm font-medium'>{t('Available models')}</h5>
            <p className='text-muted-foreground text-sm'>
              {t(
                'Choose models from the catalog. Search by model ID or group.'
              )}
            </p>
          </div>
          <Input
            aria-label={t('Search models or groups')}
            placeholder={t('Search models or groups')}
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
                      ? [
                          ...new Set([
                            ...ids,
                            ...matches.map((route) => route.id),
                          ]),
                        ]
                      : ids.filter(
                          (id) => !matches.some((route) => route.id === id)
                        )
                  )
                }
              />
              {t('Select all matches')}
            </label>
            <span
              className='text-muted-foreground text-sm tabular-nums'
              role='status'
            >
              {t('{{count}} selected', { count: selectedRoutes.length })}
            </span>
          </div>
          <div className='max-h-56 min-w-0 overflow-y-auto overscroll-contain'>
            {matches.length === 0 ? (
              <p className='text-muted-foreground py-5 text-center text-sm'>
                {t('No results found')}
              </p>
            ) : (
              <div className='grid min-w-0 gap-1 @min-[40rem]/routing:grid-cols-2'>
                {matches.map((route) => (
                  <label
                    key={route.id}
                    className='hover:bg-muted/70 flex min-h-12 min-w-0 cursor-pointer items-start gap-3 rounded-lg px-3 py-2.5'
                  >
                    <Checkbox
                      className='mt-0.5 shrink-0'
                      checked={selected.includes(route.id)}
                      disabled={disabled}
                      onCheckedChange={(checked) =>
                        setSelected((ids) =>
                          checked
                            ? [...ids, route.id]
                            : ids.filter((id) => id !== route.id)
                        )
                      }
                    />
                    <span className='min-w-0 space-y-0.5'>
                      <span className='block font-mono text-sm [overflow-wrap:anywhere]'>
                        {route.model}
                      </span>
                      <span className='text-muted-foreground block text-xs [overflow-wrap:anywhere]'>
                        {route.group}
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
      {draft.attachments.map((attachment) => {
        const route = configuration.routing.routes.find(
          (item) => item.id === attachment.route_id
        )
        const context = draft.contexts.find(
          (item) => item.upstream_model === attachment.upstream_model
        )
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
                  {route?.model ?? attachment.upstream_model}
                </span>
                <span className='text-muted-foreground flex flex-wrap gap-x-4 gap-y-1 text-xs'>
                  {route && (
                    <span>
                      {t('Group')}: {route.group}
                    </span>
                  )}
                  <span>
                    {t('Priority')}: {attachment.priority}
                  </span>
                  <span>
                    {t('Weight')}: {attachment.weight}
                  </span>
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
              <div className='flex justify-end'>
                <Button
                  type='button'
                  variant='ghost'
                  className='min-h-11'
                  disabled={disabled}
                  onClick={() =>
                    put(
                      toggleChannelAttachment(
                        draft,
                        route ?? {
                          id: attachment.route_id,
                          model: attachment.upstream_model,
                          speeds: attachment.speeds,
                        },
                        false
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
                    put(
                      updateChannelAttachment(
                        draft,
                        attachment.route_id,
                        { upstream_model: event.target.value },
                        configuration.bindings.contexts
                      )
                    )
                  }
                />
              </label>
              <div className='grid min-w-0 gap-4 @min-[40rem]/routing:grid-cols-3'>
                <IntegerField
                  label={t('Priority')}
                  value={attachment.priority}
                  max={65_535}
                  disabled={disabled}
                  onChange={(priority) =>
                    put(
                      updateChannelAttachment(draft, attachment.route_id, {
                        priority,
                      })
                    )
                  }
                />
                <IntegerField
                  label={t('Weight')}
                  value={attachment.weight}
                  max={4_294_967_295}
                  disabled={disabled}
                  onChange={(weight) =>
                    put(
                      updateChannelAttachment(draft, attachment.route_id, {
                        weight,
                      })
                    )
                  }
                />
                <NativeTargetTimeoutField
                  value={attachment.timeout_ms}
                  configuration={configuration}
                  enabled={draft.enabled}
                  disabled={disabled}
                  onChange={(timeout_ms) =>
                    put(
                      updateChannelAttachment(draft, attachment.route_id, {
                        timeout_ms,
                      })
                    )
                  }
                />
              </div>
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
                      patchContext(
                        attachment.upstream_model,
                        'max_input_tokens',
                        value
                      )
                    }
                  />
                  <IntegerField
                    label={t('Maximum output tokens')}
                    value={context?.max_output_tokens ?? ''}
                    min={tool || inputOnly ? 0 : 1}
                    max={tool || inputOnly ? 0 : 4_294_967_295}
                    disabled={disabled}
                    onChange={(value) =>
                      patchContext(
                        attachment.upstream_model,
                        'max_output_tokens',
                        value
                      )
                    }
                  />
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
                        patchContext(
                          attachment.upstream_model,
                          'contract_revision',
                          event.target.value
                        )
                      }
                    />
                  </label>
                </div>
              </details>
            </fieldset>
          </details>
        )
      })}
      {draft.attachments.length === 0 && (
        <p className='text-muted-foreground text-sm'>
          {t('No models selected')}
        </p>
      )}
      {configuration.routing.routes.length === 0 && (
        <p className='text-muted-foreground text-sm'>
          {t('No models available')}
        </p>
      )}
    </section>
  )
}
