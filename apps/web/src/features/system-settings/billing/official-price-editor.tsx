/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useQuery } from '@tanstack/react-query'
import { useMemo, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Textarea } from '@/components/ui/textarea'
import {
  microsToUsd,
  usdToMicros,
} from '@/features/channels/components/native-model-draft'
import type {
  NativeConfiguration,
  NativeRoutingScope,
} from '@/features/channels/native-api'
import { getServerErrorMessage } from '@/lib/server-error-message'

import {
  cacheWriteCoverageError,
  cacheWriteTariffTarget,
  officialPriceDestinations,
} from './official-cache-write-coverage'
import { priceRateKeys, type PriceRateKey } from './native-price-rules'
import {
  previewOfficialPriceToml,
  publishOfficialPrice,
  readOfficialPriceToml,
  syncOfficialPriceToml,
  type OfficialPricePreview,
} from './official-price-api'
import {
  missingOfficialRatePaths,
  readOfficialToml,
  writeOfficialToml,
} from './official-price-toml'

type Props = {
  scope: NativeRoutingScope
  configuration: NativeConfiguration
  model: string
  channelId?: number
  upstreamModel?: string
  onPublished?: () => void | Promise<void>
}

export function OfficialPriceEditor(props: Props) {
  const { userId, sessionId, role } = props.scope
  return (
    <Editor
      key={JSON.stringify([
        userId,
        sessionId,
        role,
        props.model,
        props.channelId,
        props.upstreamModel,
      ])}
      {...props}
    />
  )
}

function Editor({
  scope,
  configuration,
  model,
  channelId,
  upstreamModel,
  onPublished,
}: Props) {
  const { t } = useTranslation()
  const [base, setBase] = useState(configuration)
  const [draft, setDraft] = useState<string | null>(null)
  const [preview, setPreview] = useState<OfficialPricePreview | null>(null)
  const [source, setSource] = useState(channelId ? 'channel' : 'models.dev')
  const [channel, setChannel] = useState(channelId ? String(channelId) : '')
  const [provider, setProvider] = useState('')
  const [sourceModel, setSourceModel] = useState(upstreamModel ?? model)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState<'missing' | 'published' | null>(null)
  const [busy, setBusy] = useState(false)
  const pending = useRef(false)
  const fileInput = useRef<HTMLInputElement>(null)
  const query = useQuery({
    queryKey: [
      'official-model-price-toml',
      scope.userId,
      scope.sessionId,
      scope.role,
      base.version.routing,
      base.version.pricing,
      model,
    ],
    queryFn: () => readOfficialPriceToml(scope, base, model),
    retry: false,
    gcTime: 0,
    staleTime: Infinity,
  })
  const sourceChannel =
    source !== 'models.dev' && channel ? Number(channel) : undefined
  const diagnostics = useMemo(() => {
    if (draft === null) return
    let price: ReturnType<typeof readOfficialToml>
    try {
      price = readOfficialToml(model, draft)
    } catch {
      // An unfinished TOML edit can still be checked with Preview.
      return
    }
    const destinations = officialPriceDestinations(
      base.routing,
      model,
      channelId,
      sourceChannel
    )
    return {
      missing: missingOfficialRatePaths(
        price,
        cacheWriteTariffTarget(price, destinations)
      ),
      coverage: cacheWriteCoverageError(price, destinations),
    }
  }, [base.routing, channelId, draft, model, sourceChannel])
  const missingRates = diagnostics?.missing ?? []
  const text = draft ?? query.data
  const flat = useMemo(() => {
    if (text === undefined) return
    try {
      const candidate = readOfficialToml(model, text)
      if (!candidate.tiered && !candidate.unmapped_rates) return candidate
    } catch {
      // An invalid draft remains editable in the original TOML field.
    }
  }, [model, text])
  const rateLabels: Record<PriceRateKey, string> = {
    input: t('Input price (USD / 1M tokens)'),
    output: t('Output price (USD / 1M tokens)'),
    cache_read: t('Cache read (USD / 1M tokens)'),
    cache_write: t('Cache write (USD / 1M tokens)'),
    request: t('Request fee (USD / request)'),
  }
  const stale =
    configuration.version.pricing !== base.version.pricing ||
    configuration.version.routing !== base.version.routing
  const editable =
    text !== undefined && !stale && !busy && notice !== 'published'
  const changed =
    preview &&
    Object.values(preview.changes).some((models) => models.length > 0)
  const existing = Object.hasOwn(
    base.bindings.official_prices?.models ?? {},
    model
  )

  function change(next: string) {
    setDraft(next)
    setPreview(null)
    setNotice(null)
    setError('')
  }
  async function run(action: () => Promise<void>) {
    if (pending.current) return
    pending.current = true
    setBusy(true)
    setError('')
    try {
      await action()
    } catch (cause) {
      setError(
        getServerErrorMessage(cause) ??
          (cause instanceof Error ? cause.message : t('Failed to save'))
      )
    } finally {
      pending.current = false
      setBusy(false)
    }
  }
  async function sync() {
    if (text === undefined) return
    setPreview(null)
    const selection =
      source === 'models.dev'
        ? {
            kind: 'models.dev' as const,
            provider: provider.trim(),
            sourceModel: sourceModel.trim(),
          }
        : {
            kind: 'channel' as const,
            channelId: Number(channel),
            ...(Number(channel) === channelId && upstreamModel
              ? { upstreamModel }
              : {}),
            ...(source === 'openrouter'
              ? { format: 'openrouter_usd_per_token' as const }
              : {}),
          }
    const result = await syncOfficialPriceToml(scope, model, text, selection)
    if (result.found) change(result.toml)
    else setNotice('missing')
  }
  function download() {
    if (text === undefined) return
    const url = URL.createObjectURL(
      new Blob([text], { type: 'application/toml;charset=utf-8' })
    )
    const link = document.createElement('a')
    link.download = `${encodeURIComponent(model)}.toml`
    link.href = url
    link.click()
    URL.revokeObjectURL(url)
  }
  return (
    <section
      aria-busy={busy}
      aria-label={t('Official base price')}
      className='min-w-0 space-y-4'
    >
      <h3 className='font-medium break-all'>{model}</h3>
      <div className='flex flex-wrap items-end gap-2'>
        <label className='space-y-1 text-sm'>
          <span>{t('Source')}</span>
          <NativeSelect
            disabled={!editable}
            value={source}
            onChange={(event) => {
              setSource(event.target.value)
              setNotice(null)
            }}
          >
            <NativeSelectOption value='models.dev'>
              models.dev
            </NativeSelectOption>
            <NativeSelectOption value='channel'>
              {t('Channel')}
            </NativeSelectOption>
            <NativeSelectOption value='openrouter'>
              {t('OpenRouter-compatible channel (USD/token)')}
            </NativeSelectOption>
          </NativeSelect>
        </label>
        {source === 'models.dev' ? (
          <label className='space-y-1 text-sm'>
            <span>{t('Provider')}</span>
            <Input
              value={provider}
              disabled={!editable}
              onChange={(event) => {
                setProvider(event.target.value)
                setNotice(null)
              }}
            />
          </label>
        ) : (
          <label className='space-y-1 text-sm'>
            <span>{t('Channel')}</span>
            <NativeSelect
              value={channel}
              disabled={!editable || channelId !== undefined}
              onChange={(event) => {
                setChannel(event.target.value)
                setNotice(null)
              }}
            >
              <NativeSelectOption value=''>—</NativeSelectOption>
              {base.routing.upstreams.map(({ id }) => (
                <NativeSelectOption value={String(id)} key={id}>
                  {base.bindings.names[String(id)] ||
                    t('Channel {{id}}', { id })}
                </NativeSelectOption>
              ))}
            </NativeSelect>
          </label>
        )}
        {source === 'models.dev' && (
          <label className='space-y-1 text-sm'>
            <span>{t('Model ID')}</span>
            <Input
              value={sourceModel}
              disabled={!editable}
              onChange={(event) => {
                setSourceModel(event.target.value)
                setNotice(null)
              }}
            />
          </label>
        )}
        <Button
          type='button'
          variant='outline'
          disabled={
            !editable ||
            (source === 'models.dev'
              ? !provider.trim() || !sourceModel.trim()
              : !channel)
          }
          onClick={() => void run(sync)}
        >
          {t('Fetch upstream prices')}
        </Button>
      </div>
      {notice === 'missing' && (
        <div role='status' className='flex flex-wrap items-center gap-2'>
          <p>{t('No matching upstream prices')}</p>
          {source !== 'models.dev' && (
            <Button
              type='button'
              variant='outline'
              onClick={() => {
                setSource('models.dev')
                setNotice(null)
              }}
            >
              models.dev
            </Button>
          )}
        </div>
      )}
      {query.isPending && <p role='status'>{t('Loading...')}</p>}
      {query.isError && (
        <div role='alert' className='flex items-center gap-3'>
          {getServerErrorMessage(query.error) ?? t('Failed to load data')}
          <Button
            type='button'
            variant='outline'
            onClick={() => void query.refetch()}
          >
            {t('Retry')}
          </Button>
        </div>
      )}
      {text !== undefined && (
        <>
          {flat && (
            <form
              key={text}
              className='space-y-3'
              onSubmit={(event) => {
                event.preventDefault()
                const fields = new FormData(event.currentTarget)
                const rates: typeof flat.rates = {}
                for (const key of priceRateKeys) {
                  const field = String(fields.get(key) ?? '')
                  if (!field) continue
                  const value = usdToMicros(field)
                  if (value === null) {
                    setError(rateLabels[key] + ' · ' + t('Invalid amount'))
                    return
                  }
                  rates[key] = value
                }
                change(writeOfficialToml(model, { ...flat, rates }))
              }}
            >
              <div className='grid gap-x-4 gap-y-3 sm:grid-cols-2'>
                {priceRateKeys.map((key) => (
                  <label key={key} className='block min-w-0 space-y-1 text-sm'>
                    <span>{rateLabels[key]}</span>
                    <Input
                      name={key}
                      inputMode='decimal'
                      pattern='[0-9]+([.][0-9]{1,6})?'
                      defaultValue={
                        flat.rates[key] === undefined
                          ? ''
                          : microsToUsd(flat.rates[key])
                      }
                      readOnly={!editable}
                    />
                  </label>
                ))}
              </div>
              <Button type='submit' variant='outline' disabled={!editable}>
                {t('Apply')}
              </Button>
            </form>
          )}
          <details className='space-y-2' open={!flat}>
            <summary className='cursor-pointer text-sm font-medium'>
              {t('Advanced')} · {t('Model price (TOML)')}
            </summary>
            <Textarea
              aria-label={t('Model price (TOML)')}
              className='min-h-48 font-mono'
              spellCheck={false}
              value={text}
              readOnly={!editable}
              onChange={(event) => change(event.target.value)}
            />
          </details>
          {(missingRates.length > 0 || diagnostics?.coverage) && (
            <div role='status' className='space-y-1 text-sm'>
              {missingRates.length > 0 && (
                <>
                  <p>{t('Missing price fields')}</p>
                  <ul className='max-h-48 list-disc overflow-y-auto ps-5'>
                    {missingRates.slice(0, 64).map((path) => (
                      <li key={path}>
                        <code className='break-all'>{path}</code>
                      </li>
                    ))}
                    {missingRates.length > 64 && (
                      <li>+{missingRates.length - 64}</li>
                    )}
                  </ul>
                </>
              )}
              {diagnostics?.coverage && <p>{t(diagnostics.coverage)}</p>}
            </div>
          )}
          <div className='flex flex-wrap items-center gap-2'>
            <input
              ref={fileInput}
              type='file'
              accept='.toml'
              hidden
              onChange={(event) => {
                const file = event.target.files?.[0]
                event.target.value = ''
                if (file) void run(async () => change(await file.text()))
              }}
            />
            <Button
              type='button'
              variant='outline'
              disabled={!editable}
              onClick={() => fileInput.current?.click()}
            >
              {t('Import file')}
            </Button>
            <Button type='button' variant='outline' onClick={download}>
              {t('Download')}
            </Button>
            {existing && (
              <Button
                type='button'
                variant='ghost'
                disabled={!editable}
                onClick={() =>
                  void run(async () => {
                    setPreview(null)
                    setPreview(
                      await previewOfficialPriceToml(scope, base, model, null)
                    )
                  })
                }
              >
                {t('Preview price removal')}
              </Button>
            )}
            <Button
              type='button'
              disabled={
                !editable ||
                text === query.data ||
                missingRates.length > 0 ||
                Boolean(diagnostics?.coverage)
              }
              onClick={() =>
                void run(async () => {
                  setPreview(null)
                  setPreview(
                    await previewOfficialPriceToml(
                      scope,
                      base,
                      model,
                      text,
                      channelId,
                      sourceChannel
                    )
                  )
                })
              }
            >
              {t('Preview changes')}
            </Button>
          </div>
        </>
      )}
      {preview && !stale && (
        <div className='space-y-3'>
          {changed ? (
            <dl className='space-y-1 text-sm'>
              {(
                [
                  [t('New'), preview.changes.added],
                  [t('Updated'), preview.changes.modified],
                  [t('Removed'), preview.changes.deleted],
                ] as const
              ).map(
                ([label, models]) =>
                  models.length > 0 && (
                    <div key={label} className='flex flex-wrap gap-2'>
                      <dt className='font-medium'>{label}</dt>
                      {models.map((id) => (
                        <dd key={id} className='font-mono break-all'>
                          {id}
                        </dd>
                      ))}
                    </div>
                  )
              )}
            </dl>
          ) : (
            <p role='status'>{t('No changes')}</p>
          )}
          {changed && (
            <Button
              type='button'
              disabled={!editable}
              onClick={() =>
                void run(async () => {
                  await publishOfficialPrice(scope, preview)
                  setPreview(null)
                  setNotice('published')
                  await onPublished?.()
                })
              }
            >
              {t('Publish prices')}
            </Button>
          )}
        </div>
      )}
      {notice === 'published' && <p role='status'>{t('Published')}</p>}
      {stale && (
        <div className='space-y-2'>
          {notice !== 'published' && (
            <p role='status'>
              {t(
                'Published configuration changed. Keep your draft for reference, reload the latest prices and review your changes again.'
              )}
            </p>
          )}
          <Button
            type='button'
            variant='outline'
            disabled={busy}
            onClick={() => {
              setBase(configuration)
              setDraft(null)
              setPreview(null)
              setError('')
              setNotice(null)
            }}
          >
            {t('Reload saved version')}
          </Button>
        </div>
      )}
      {error && (
        <p role='alert' className='text-destructive'>
          {t(error)}
        </p>
      )}
    </section>
  )
}
