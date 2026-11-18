/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
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
  const [selected, setSelected] = useState('')
  const draft = readChannelDraft(document)
  if (!draft) return null
  const current = draft
  const put = (next: NativeChannelDraft) =>
    onChange(JSON.stringify(next, null, 2))
  const available = configuration.routing.routes.filter(
    (route) => !draft.attachments.some((item) => item.route_id === route.id)
  )
  const selectedRoute = available.find((route) => String(route.id) === selected)
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
          <fieldset
            key={attachment.route_id}
            className='min-w-0 space-y-4 border-t pt-4'
            data-route-id={attachment.route_id}
          >
            <legend className='sr-only'>
              {route?.model ?? attachment.upstream_model}
            </legend>
            <div className='flex min-w-0 flex-wrap items-start justify-between gap-3'>
              <div className='min-w-0 space-y-1'>
                <h5 className='font-mono text-sm font-medium break-all'>
                  {route?.model ?? attachment.upstream_model}
                </h5>
                {route && (
                  <p className='text-muted-foreground text-sm break-all'>
                    {t('Group')}: {route.group}
                  </p>
                )}
              </div>
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
        )
      })}
      {draft.attachments.length === 0 && (
        <p className='text-muted-foreground text-sm'>
          {t('No models selected')}
        </p>
      )}
      {available.length > 0 ? (
        <div className='flex min-w-0 flex-wrap items-end gap-3'>
          <label className={`${fieldClass} flex-1 basis-56`}>
            <span>{t('Available models')}</span>
            <select
              className='bg-muted/65 min-h-11 w-full min-w-0 rounded-lg px-3 text-base'
              value={selectedRoute ? selected : ''}
              disabled={disabled}
              onChange={(event) => setSelected(event.target.value)}
            >
              <option value=''>{t('Select a model ID')}</option>
              {available.map((route) => (
                <option key={route.id} value={route.id}>
                  {route.model} · {route.group}
                </option>
              ))}
            </select>
          </label>
          <Button
            type='button'
            variant='outline'
            className='min-h-11'
            disabled={disabled || !selectedRoute}
            onClick={() => {
              if (!selectedRoute) return
              put(
                toggleChannelAttachment(
                  draft,
                  selectedRoute,
                  true,
                  configuration
                )
              )
              setSelected('')
            }}
          >
            {t('Add Model')}
          </Button>
        </div>
      ) : configuration.routing.routes.length === 0 ? (
        <p className='text-muted-foreground text-sm'>
          {t('No models available')}
        </p>
      ) : null}
    </section>
  )
}
