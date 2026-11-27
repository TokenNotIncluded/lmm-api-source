/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { cacheWriteCoverageError } from '@/features/system-settings/billing/official-cache-write-coverage'
import { getServerErrorMessage } from '@/lib/server-error-message'

import { newNativeContext } from '../lib/native-routing-form'
import { newNativeTargetTimeoutMs } from '../lib/native-target-timeout'
import {
  NATIVE_ROOT,
  nativeRequest,
  type NativeConfiguration,
  type NativeRoutingScope,
  type NativeUpstream,
} from '../native-api'
import { defaultModelDisplayName } from './native-model-draft'

export function NativePendingRoute({
  scope,
  configuration,
  channel,
  model,
  upstreamModel,
  onPublished,
}: {
  scope: NativeRoutingScope
  configuration: NativeConfiguration
  channel: NativeUpstream
  model: string
  upstreamModel: string
  onPublished: () => Promise<void>
}) {
  const { t } = useTranslation()
  const { routing, bindings, version } = configuration
  const groups = routing.groups.filter((item) => item.enabled)
  const [group, setGroup] = useState(groups[0]?.name ?? '')
  const [inputLimit, setInputLimit] = useState('')
  const [outputLimit, setOutputLimit] = useState('')
  const [revision, setRevision] = useState('')
  const [digest, setDigest] = useState('')
  const [busy, setBusy] = useState(false)
  const [published, setPublished] = useState(false)
  const [error, setError] = useState('')
  const locked = useRef(false)
  const existing = routing.routes.find(
    (route) => route.model === model && route.group === group
  )
  const matches =
    existing?.targets.filter((target) => target.upstream_id === channel.id) ??
    []
  const previous = matches[0]
  const conflict =
    matches.length > 1 ||
    (previous !== undefined && previous.upstream_model !== upstreamModel)
  const context = bindings.contexts.find(
    (item) =>
      item.upstream_id === channel.id && item.upstream_model === upstreamModel
  )
  const tool = ['deepseek_search', 'deepseek_fetch'].includes(channel.protocol)
  const inputOnly =
    tool || ['openai_embeddings', 'jina_rerank'].includes(channel.protocol)
  const needsContext = !context
  const maxInput = tool ? 0 : Number(inputLimit)
  const maxOutput = inputOnly ? 0 : Number(outputLimit)
  const validContext =
    !needsContext ||
    (Number.isSafeInteger(maxInput) &&
      (tool || maxInput > 0) &&
      Number.isSafeInteger(maxOutput) &&
      (inputOnly || maxOutput > 0) &&
      !!revision.trim())
  const priceIssue = cacheWriteCoverageError(
    bindings.official_prices?.models[model],
    [{ protocol: channel.protocol, payload: previous?.payload }]
  )
  const ready =
    !priceIssue &&
    channel.enabled &&
    !!group &&
    existing?.enabled !== false &&
    !conflict &&
    !previous?.enabled &&
    validContext &&
    !published

  function proposal() {
    const routeId =
      existing?.id ??
      routing.routes.reduce((max, item) => Math.max(max, item.id), 0) + 1
    const targetId =
      previous?.id ??
      routing.routes.reduce(
        (max, item) =>
          item.targets.reduce((n, target) => Math.max(n, target.id), max),
        0
      ) + 1
    const speed = existing?.speeds.includes('standard')
      ? 'standard'
      : (existing?.speeds[0] ?? 'standard')
    const target = previous
      ? { ...previous, enabled: true }
      : {
          id: targetId,
          upstream_id: channel.id,
          upstream_model: upstreamModel,
          enabled: true,
          priority: 0,
          weight: 1,
          speeds: [speed],
          timeout_ms: newNativeTargetTimeoutMs(configuration),
          payload: 'adapt',
        }
    const route = existing
      ? {
          ...existing,
          targets: previous
            ? existing.targets.map((item) =>
                item.id === previous.id ? target : item
              )
            : [...existing.targets, target],
        }
      : {
          id: routeId,
          model,
          group,
          enabled: true,
          speeds: ['standard'],
          targets: [target],
          failover: {
            max_attempts: 1,
            retry_on: [],
            allow_replay_after_send: false,
          },
        }
    const metadata = bindings.models[routeId] ?? {
      display_name: defaultModelDisplayName(model),
      description: '',
      vendor_id: null,
    }
    const contexts = needsContext
      ? [
          {
            ...newNativeContext(channel, upstreamModel),
            max_input_tokens: maxInput,
            max_output_tokens: maxOutput,
            contract_revision: revision.trim(),
          },
        ]
      : []
    return { expected_version: version, route, metadata, contexts, prices: [] }
  }
  async function submit(publish: boolean) {
    if (locked.current || !ready || (publish && !digest)) return
    locked.current = true
    setBusy(true)
    setError('')
    try {
      const input = proposal()
      if (publish) {
        await nativeRequest(
          scope,
          existing ? 'PUT' : 'POST',
          existing
            ? `${NATIVE_ROOT}/models/${existing.id}`
            : `${NATIVE_ROOT}/models`,
          { ...input, confirmation_digest: digest }
        )
        setPublished(true)
        setDigest('')
        await onPublished()
      } else {
        setDigest('')
        const receipt = await nativeRequest<{ confirmation_digest: string }>(
          scope,
          'POST',
          `${NATIVE_ROOT}/models/preview`,
          { editing: !!existing, model: input }
        )
        if (!/^[0-9a-f]{64}$/.test(receipt.confirmation_digest)) {
          throw new Error(t('Failed to load data'))
        }
        setDigest(receipt.confirmation_digest)
      }
    } catch (cause) {
      setDigest('')
      setError(getServerErrorMessage(cause) ?? t('Failed to save'))
    } finally {
      locked.current = false
      setBusy(false)
    }
  }
  return (
    <fieldset
      disabled={busy || published}
      className='space-y-3'
      onChange={() => {
        setDigest('')
        setError('')
      }}
    >
      <label className='grid gap-1 text-sm'>
        {t('Group')}
        <select
          className='border-input bg-background rounded-md border px-3 py-2'
          value={group}
          onChange={(e) => setGroup(e.target.value)}
        >
          {groups.map((item) => (
            <option key={item.name}>{item.name}</option>
          ))}
        </select>
      </label>
      {conflict ? (
        <p role='status'>
          {t('This channel already has a different target in this route.')}
        </p>
      ) : !channel.enabled || existing?.enabled === false ? (
        <p role='status'>{t('Disabled')}</p>
      ) : (
        needsContext && (
          <div className='grid gap-2 sm:grid-cols-2'>
            {!tool && (
              <label className='grid gap-1 text-sm'>
                {t('Maximum input tokens')}
                <Input
                  type='number'
                  min={1}
                  step={1}
                  value={inputLimit}
                  onChange={(e) => setInputLimit(e.target.value)}
                />
              </label>
            )}
            {!inputOnly && (
              <label className='grid gap-1 text-sm'>
                {t('Maximum output tokens')}
                <Input
                  type='number'
                  min={1}
                  step={1}
                  value={outputLimit}
                  onChange={(e) => setOutputLimit(e.target.value)}
                />
              </label>
            )}
            <label className='grid gap-1 text-sm'>
              {t('Contract revision')}
              <Input
                value={revision}
                onChange={(e) => setRevision(e.target.value)}
              />
            </label>
          </div>
        )
      )}
      {priceIssue && (
        <p role='alert' className='text-destructive text-sm'>
          {t(priceIssue)}
        </p>
      )}
      {digest && (
        <p className='text-sm [overflow-wrap:anywhere]' role='status'>
          <code>
            {group} / {model} → #{channel.id} / {upstreamModel}
          </code>
          {' · '}
          {previous ? t('Disabled') : '—'} → {t('Enabled')}
        </p>
      )}
      {error && (
        <p role='alert' className='text-destructive text-sm'>
          {error}
        </p>
      )}
      <div className='flex gap-2'>
        <Button
          type='button'
          variant='outline'
          disabled={!ready || busy}
          onClick={() => void submit(false)}
        >
          {t('Preview route')}
        </Button>
        <Button
          type='button'
          disabled={!ready || busy || !digest}
          onClick={() => void submit(true)}
        >
          {t('Publish route')}
        </Button>
      </div>
    </fieldset>
  )
}
