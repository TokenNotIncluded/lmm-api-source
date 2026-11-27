/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'

import {
  stringifyNativeDocument,
  type NativeConfiguration,
} from '../native-api'
import { NativeFailoverFields } from './native-failover-fields'
import {
  defaultModelDisplayName,
  maxGroupMultiplierPpm,
  microsToUsd,
  modelIdentity,
  newModelTarget,
  readNativeModelDraft,
  replaceModelTargets,
  usdToMicros,
  type NativeModelDraft,
  type NativeTargetDraft,
} from './native-model-draft'
import { NativeTargetTimeoutField } from './native-target-timeout-field'

const fieldClass = 'block min-w-0 space-y-2 text-sm'
const selectClass =
  'bg-background border-input min-h-11 w-full min-w-0 rounded-md border px-3 text-base'

function IntegerField({
  label,
  value,
  disabled,
  onChange,
  min = 0,
  max = Number.MAX_SAFE_INTEGER,
}: {
  label: string
  value: number | string
  disabled: boolean
  onChange: (value: number | string) => void
  min?: number
  max?: number
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
        className='min-h-11'
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

function PriceField({
  label,
  value,
  disabled,
  onChange,
  multiplier = false,
}: {
  label: string
  value: number | string
  disabled: boolean
  onChange: (value: number | string) => void
  multiplier?: boolean
}) {
  const { t } = useTranslation()
  const [text, setText] = useState(() => microsToUsd(value))
  const emitted = useRef(value)
  const input = useRef<HTMLInputElement>(null)
  const max = multiplier ? maxGroupMultiplierPpm : Number.MAX_SAFE_INTEGER
  const invalid = multiplier
    ? t('Invalid cost multiplier')
    : t('Enter a non-negative price with at most six decimal places.')
  useEffect(() => {
    if (value !== emitted.current) setText(microsToUsd(value))
    emitted.current = value
    const amount = usdToMicros(microsToUsd(value))
    input.current?.setCustomValidity(
      amount === null || amount > max ? invalid : ''
    )
  }, [value, max, invalid])
  return (
    <label className={fieldClass}>
      <span>{label}</span>
      <Input
        type='text'
        ref={input}
        inputMode='decimal'
        required
        pattern='[0-9]+(\.[0-9]{0,6})?'
        className='min-h-11 tabular-nums'
        value={text}
        disabled={disabled}
        onChange={(event) => {
          const raw = event.target.value
          const amount = usdToMicros(raw)
          const valid = amount !== null && amount <= max
          event.target.setCustomValidity(valid ? '' : invalid)
          setText(raw)
          emitted.current = valid ? amount : raw
          onChange(emitted.current)
        }}
      />
    </label>
  )
}

export function NativeModelFields({
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
  const [tagText, setTagText] = useState('')
  const draft = readNativeModelDraft(document)
  if (!draft) return null
  const displayName = draft.metadata.display_name
  const displayNameError = !displayName.trim()
    ? t('Name is required')
    : /\p{Cc}/u.test(displayName)
      ? t('Display name cannot contain control characters')
      : defaultModelDisplayName(displayName) !== displayName
        ? t('Display name must fit within 128 UTF-8 bytes.')
        : ''
  const current = draft
  const usedUpstreams = new Set(
    draft.route.targets.map((target) => target.upstream_id)
  )
  const nextUpstream = configuration.routing.upstreams.find(
    (upstream) => !usedUpstreams.has(upstream.id)
  )
  const put = (next: NativeModelDraft) =>
    onChange(stringifyNativeDocument(next, 2))
  function patchTarget(index: number, patch: Partial<NativeTargetDraft>) {
    const targets = current.route.targets.map((target, position) =>
      position === index ? { ...target, ...patch } : target
    )
    put(
      'upstream_id' in patch || 'upstream_model' in patch
        ? replaceModelTargets(current, targets, configuration)
        : { ...current, route: { ...current.route, targets } }
    )
  }
  function patchContext(
    target: NativeTargetDraft,
    key: string,
    value: string | number
  ) {
    const next = replaceModelTargets(
      current,
      current.route.targets,
      configuration
    )
    next.contexts = next.contexts.map((context) =>
      context.upstream_id === target.upstream_id &&
      context.upstream_model === target.upstream_model
        ? { ...context, [key]: value }
        : context
    )
    put(next)
  }
  const rates = [
    ['input', t('Input price (USD / 1M tokens)')],
    ['output', t('Output price (USD / 1M tokens)')],
    ['cache_read', t('Cache read (USD / 1M tokens)')],
    ['cache_write', t('Cache write (USD / 1M tokens)')],
    ['request', t('Request fee (USD / request)')],
  ] as const
  return (
    <div className='min-w-0 space-y-7' data-slot='native-model-fields'>
      <div className='grid min-w-0 gap-4 @min-[40rem]/routing:grid-cols-2'>
        <label className={fieldClass}>
          <span>{t('Model name')}</span>
          <Input
            className='min-h-11 font-mono'
            value={draft.route.model}
            maxLength={200}
            required
            disabled={disabled}
            onChange={(event) =>
              put(modelIdentity(draft, 'model', event.target.value))
            }
          />
        </label>
        <label className={fieldClass}>
          <span>{t('Group')}</span>
          <select
            className={selectClass}
            value={draft.route.group}
            required
            disabled={disabled}
            onChange={(event) =>
              put(
                modelIdentity(
                  draft,
                  'group',
                  event.target.value,
                  configuration.prices.rules
                )
              )
            }
          >
            {configuration.routing.groups.map((group) => (
              <option key={group.name} value={group.name}>
                {group.name}
              </option>
            ))}
          </select>
        </label>
        <label className={fieldClass}>
          <span>{t('Display name')}</span>
          <Input
            className='min-h-11'
            value={displayName}
            required
            aria-invalid={displayName && displayNameError ? true : undefined}
            ref={(input) => input?.setCustomValidity(displayNameError)}
            disabled={disabled}
            onChange={(event) =>
              put({
                ...draft,
                metadata: {
                  ...draft.metadata,
                  display_name: event.target.value,
                },
              })
            }
          />
          {displayName && displayNameError && (
            <span className='text-destructive text-xs'>{displayNameError}</span>
          )}
        </label>
        <label className={fieldClass}>
          <span>{t('Vendor')}</span>
          <select
            className={selectClass}
            value={draft.metadata.vendor_id ?? ''}
            disabled={disabled}
            onChange={(event) =>
              put({
                ...draft,
                metadata: {
                  ...draft.metadata,
                  vendor_id: event.target.value
                    ? Number(event.target.value)
                    : null,
                },
              })
            }
          >
            <option value=''>{t('None')}</option>
            {Object.values(configuration.bindings.vendors).map((vendor) => (
              <option key={vendor.id} value={vendor.id}>
                {vendor.name}
              </option>
            ))}
          </select>
        </label>
        <label className={`${fieldClass} @min-[40rem]/routing:col-span-2`}>
          <span>{t('Description')}</span>
          <Textarea
            value={draft.metadata.description}
            disabled={disabled}
            onChange={(event) =>
              put({
                ...draft,
                metadata: {
                  ...draft.metadata,
                  description: event.target.value,
                },
              })
            }
          />
        </label>
        <fieldset className='min-w-0 space-y-2 @min-[40rem]/routing:col-span-2'>
          <legend className='text-sm'>{t('Tags')}</legend>
          <div className='flex flex-wrap gap-1.5'>
            {(draft.metadata.tags ?? []).map((tag) => (
              <Button
                key={tag}
                type='button'
                variant='secondary'
                size='sm'
                title={t('Remove tag')}
                disabled={disabled}
                onClick={() =>
                  put({
                    ...draft,
                    metadata: {
                      ...draft.metadata,
                      tags: draft.metadata.tags?.filter((item) => item !== tag),
                    },
                  })
                }
              >
                {tag} <span aria-hidden='true'>×</span>
              </Button>
            ))}
          </div>
          <div className='flex items-center gap-2'>
            <Input
              aria-label={t('Tag')}
              maxLength={128}
              value={tagText}
              disabled={disabled}
              onChange={(event) => setTagText(event.target.value)}
            />
            <Button
              type='button'
              variant='outline'
              disabled={
                disabled ||
                !tagText.trim() ||
                (draft.metadata.tags?.length ?? 0) >= 20 ||
                draft.metadata.tags?.includes(tagText.trim())
              }
              onClick={() => {
                put({
                  ...draft,
                  metadata: {
                    ...draft.metadata,
                    tags: [...(draft.metadata.tags ?? []), tagText.trim()],
                  },
                })
                setTagText('')
              }}
            >
              {t('Add')}
            </Button>
          </div>
        </fieldset>
        <label className='flex min-h-11 items-center gap-3 text-sm'>
          <Checkbox
            checked={draft.route.enabled}
            disabled={disabled}
            onCheckedChange={(checked) =>
              put({
                ...draft,
                route: { ...draft.route, enabled: checked === true },
              })
            }
          />
          {t('Enabled')}
        </label>
      </div>
      <section className='space-y-4' aria-label={t('Routing targets')}>
        <div className='flex flex-wrap items-center justify-between gap-3'>
          <h4 className='font-medium'>{t('Routing targets')}</h4>
          <Button
            type='button'
            variant='outline'
            className='min-h-11'
            disabled={disabled || !nextUpstream}
            onClick={() => {
              if (!nextUpstream) return
              put(
                replaceModelTargets(
                  draft,
                  [
                    ...draft.route.targets,
                    newModelTarget(draft, configuration, nextUpstream.id),
                  ],
                  configuration
                )
              )
            }}
          >
            {t('Add target')}
          </Button>
        </div>
        <p className='text-muted-foreground text-sm'>
          {t(
            'Lower priority values run first. Weight distributes requests within the same priority.'
          )}
        </p>
        {draft.route.targets.map((target, index) => {
          const context =
            draft.contexts.find(
              (item) =>
                item.upstream_id === target.upstream_id &&
                item.upstream_model === target.upstream_model
            ) ??
            configuration.bindings.contexts.find(
              (item) =>
                item.upstream_id === target.upstream_id &&
                item.upstream_model === target.upstream_model
            )
          const contextNumber = (key: string) => {
            const value = context?.[key]
            return typeof value === 'number' || typeof value === 'string'
              ? value
              : ''
          }
          const publishedContract = configuration.bindings.contexts.some(
            (item) =>
              item.upstream_id === target.upstream_id &&
              item.upstream_model === target.upstream_model &&
              item.contract_revision !== ''
          )
          return (
            <fieldset
              key={target.id}
              className='min-w-0 space-y-4 border-t pt-4'
            >
              <legend className='sr-only'>
                {t('Routing target {{number}}', { number: index + 1 })}
              </legend>
              <div className='grid min-w-0 gap-4 @min-[40rem]/routing:grid-cols-2'>
                <label className={fieldClass}>
                  <span>{t('Channel')}</span>
                  <select
                    className={selectClass}
                    value={target.upstream_id}
                    disabled={disabled}
                    onChange={(event) =>
                      patchTarget(index, {
                        upstream_id: Number(event.target.value),
                      })
                    }
                  >
                    {configuration.routing.upstreams.map((upstream) => (
                      <option
                        key={upstream.id}
                        value={upstream.id}
                        disabled={
                          upstream.id !== target.upstream_id &&
                          usedUpstreams.has(upstream.id)
                        }
                      >
                        {configuration.bindings.names[upstream.id] ||
                          upstream.endpoint}
                      </option>
                    ))}
                  </select>
                </label>
                <label className={fieldClass}>
                  <span>{t('Upstream model')}</span>
                  <Input
                    className='min-h-11 font-mono'
                    value={target.upstream_model}
                    required
                    maxLength={200}
                    disabled={disabled}
                    onChange={(event) =>
                      patchTarget(index, { upstream_model: event.target.value })
                    }
                  />
                </label>
                <IntegerField
                  label={t('Priority')}
                  value={target.priority}
                  max={65_535}
                  disabled={disabled}
                  onChange={(priority) => patchTarget(index, { priority })}
                />
                <IntegerField
                  label={t('Weight')}
                  value={target.weight}
                  max={4_294_967_295}
                  disabled={disabled}
                  onChange={(weight) => patchTarget(index, { weight })}
                />
                <NativeTargetTimeoutField
                  value={target.timeout_ms}
                  configuration={configuration}
                  enabled={configuration.routing.upstreams.some(
                    (item) => item.id === target.upstream_id && item.enabled
                  )}
                  disabled={disabled}
                  onChange={(timeout_ms) => patchTarget(index, { timeout_ms })}
                />
                <label className='flex min-h-11 items-center gap-3 text-sm'>
                  <Checkbox
                    checked={target.enabled}
                    disabled={disabled}
                    onCheckedChange={(checked) =>
                      patchTarget(index, { enabled: checked === true })
                    }
                  />
                  {t('Enabled')}
                </label>
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
                    value={contextNumber('max_input_tokens')}
                    max={4_294_967_295}
                    disabled={disabled}
                    onChange={(value) =>
                      patchContext(target, 'max_input_tokens', value)
                    }
                  />
                  <IntegerField
                    label={t('Maximum output tokens')}
                    value={contextNumber('max_output_tokens')}
                    max={4_294_967_295}
                    disabled={disabled}
                    onChange={(value) =>
                      patchContext(target, 'max_output_tokens', value)
                    }
                  />
                  <label
                    className={`${fieldClass} @min-[40rem]/routing:col-span-2`}
                  >
                    <span>{t('Contract revision')}</span>
                    <Input
                      className='min-h-11'
                      value={
                        typeof context?.contract_revision === 'string'
                          ? context.contract_revision
                          : ''
                      }
                      required
                      disabled={disabled}
                      onChange={(event) =>
                        patchContext(
                          target,
                          'contract_revision',
                          event.target.value
                        )
                      }
                    />
                  </label>
                </div>
              </details>
              <Button
                type='button'
                variant='ghost'
                className='min-h-11'
                disabled={disabled}
                onClick={() =>
                  put(
                    replaceModelTargets(
                      draft,
                      draft.route.targets.filter(
                        (_, position) => position !== index
                      ),
                      configuration
                    )
                  )
                }
              >
                {t('Remove target')}
              </Button>
            </fieldset>
          )
        })}
      </section>
      <NativeFailoverFields
        value={draft.route.failover}
        disabled={disabled}
        onChange={(failover) =>
          put({ ...draft, route: { ...draft.route, failover } })
        }
      />
      <section className='space-y-5' aria-label={t('Pricing')}>
        <h4 className='font-medium'>{t('Pricing')}</h4>
        {draft.prices.map((price, index) => (
          <fieldset key={price.id} className='min-w-0 space-y-4'>
            <legend className='text-muted-foreground mb-3 text-sm'>
              {price.speed}
            </legend>
            <PriceField
              label={t('Group multiplier')}
              value={price.group_multiplier_ppm}
              multiplier
              disabled={disabled}
              onChange={(value) =>
                put({
                  ...draft,
                  prices: draft.prices.map((item, position) =>
                    position === index
                      ? { ...item, group_multiplier_ppm: value }
                      : item
                  ),
                })
              }
            />
            {price.billing_mode && price.billing_mode !== 'tokens' ? (
              <p className='text-muted-foreground text-sm'>
                {t('Edit advanced pricing in Advanced Settings.')}
              </p>
            ) : (
              <div className='grid min-w-0 gap-4 @min-[40rem]/routing:grid-cols-2'>
                {rates.map(([key, label]) => (
                  <PriceField
                    key={key}
                    label={label}
                    value={price.rates[key]}
                    disabled={disabled}
                    onChange={(value) =>
                      put({
                        ...draft,
                        prices: draft.prices.map((item, position) =>
                          position === index
                            ? {
                                ...item,
                                rates: { ...item.rates, [key]: value },
                              }
                            : item
                        ),
                      })
                    }
                  />
                ))}
              </div>
            )}
          </fieldset>
        ))}
      </section>
    </div>
  )
}
