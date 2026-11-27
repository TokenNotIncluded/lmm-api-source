/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'
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
import {
  fetchModelsDevCatalog,
  officialModelsDevQuote,
} from './official-models-dev-quote'
import {
  previewMissingOfficialPrices,
  publishOfficialPrice,
  type OfficialPricePreview,
} from './official-price-api'
import {
  completeOfficialPrice,
  mergeQuote,
  missingOfficialRatePaths,
  readOfficialToml,
  writeOfficialToml,
} from './official-price-toml'

type Props = {
  scope: NativeRoutingScope
  configuration: NativeConfiguration
  models: string[]
  channelId?: number
  onPublished: () => Promise<unknown> | void
  onClose?: () => void
}
type Row = {
  model: string
  sourceModel: string
  provider: string
  providers: string[]
  toml: string
  selected: boolean
  error?: string
}
function failure(error: unknown) {
  return (
    getServerErrorMessage(error) ??
    (error instanceof Error ? error.message : 'Failed to load data')
  )
}
function hasModel(source: unknown, model: string) {
  if (!source || typeof source !== 'object' || !('models' in source))
    return false
  const models = source.models
  return !!models && typeof models === 'object' && Object.hasOwn(models, model)
}
export async function batchQuoteRow(
  catalog: unknown,
  model: string,
  sourceModel = model,
  preferred = ''
): Promise<Row> {
  const providers = Object.entries(
    catalog && typeof catalog === 'object' ? catalog : {}
  )
    .filter(([, source]) => hasModel(source, sourceModel))
    .map(([id]) => id)
    .sort()
  const provider = providers.includes(preferred)
    ? preferred
    : providers.length === 1
      ? providers[0]
      : ''
  const row: Row = {
    model,
    sourceModel,
    providers,
    provider,
    toml: '',
    selected: false,
  }
  if (!provider)
    return {
      ...row,
      error: providers.length
        ? 'Select provider'
        : 'No matching upstream prices',
    }
  try {
    const quote = officialModelsDevQuote(catalog, sourceModel, provider)
    if (!quote) return { ...row, error: 'No matching upstream prices' }
    const toml = await mergeQuote(model, writeOfficialToml(model), {
      ...quote,
      model,
      source: {
        kind: 'models_dev',
        provider,
        ...(sourceModel !== model ? { source_model: sourceModel } : {}),
      },
    })
    return { ...row, toml, selected: true }
  } catch (cause) {
    return { ...row, error: failure(cause) }
  }
}
function issues(
  row: Row,
  configuration: NativeConfiguration,
  channelId?: number
): string[] {
  if (!row.toml) return [row.error ?? 'No matching upstream prices']
  try {
    const price = readOfficialToml(row.model, row.toml)
    const targets = officialPriceDestinations(
      configuration.routing,
      row.model,
      channelId
    )
    const missing = missingOfficialRatePaths(
      price,
      cacheWriteTariffTarget(price, targets)
    )
    const coverage = cacheWriteCoverageError(price, targets)
    if (coverage) missing.push(coverage)
    if (price.unmapped_rates)
      missing.push(
        ...Object.keys(price.unmapped_rates).map(
          (key) => 'unmapped_rates.' + key
        )
      )
    if (!missing.length) completeOfficialPrice(price)
    return missing
  } catch {
    return ['Invalid model price file']
  }
}

export function OfficialBatchPriceReview(props: Props) {
  const models = [...new Set(props.models)].sort()
  return (
    <BatchReview
      key={JSON.stringify([
        props.scope.userId,
        props.scope.sessionId,
        props.scope.role,
        props.configuration.version,
        props.channelId,
        models,
      ])}
      {...props}
      models={models}
    />
  )
}
function BatchReview({
  scope,
  configuration,
  models,
  channelId,
  onPublished,
  onClose,
}: Props) {
  const { t } = useTranslation()
  const [rows, setRows] = useState<Row[]>([])
  const [catalog, setCatalog] = useState<unknown>()
  const [preview, setPreview] = useState<OfficialPricePreview>()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [published, setPublished] = useState(false)
  const [initialized, setInitialized] = useState(false)
  const [page, setPage] = useState(0)
  const inflight = useRef(false)
  const controller = useRef<AbortController | undefined>(undefined)
  useEffect(() => () => controller.current?.abort(), [])
  const existing = configuration.bindings.official_prices?.models ?? {}
  const missingModels = models.filter(
    (model) => !Object.hasOwn(existing, model)
  )
  const selected = rows.filter((row) => row.selected)
  const rowIssues = new Map(
    rows.map((row) => [row.model, issues(row, configuration, channelId)])
  )
  const change = (model: string, values: Partial<Row>) => {
    setRows((current) =>
      current.map((row) => (row.model === model ? { ...row, ...values } : row))
    )
    setPreview(undefined)
  }
  const run = async (fn: () => Promise<void>) => {
    if (inflight.current) return
    inflight.current = true
    setBusy(true)
    setError('')
    try {
      await fn()
    } catch (cause) {
      if (!controller.current?.signal.aborted) setError(failure(cause))
    } finally {
      inflight.current = false
      setBusy(false)
    }
  }
  async function load() {
    controller.current = new AbortController()
    const source = await fetchModelsDevCatalog(controller.current.signal)
    const proposals: Row[] = []
    for (let offset = 0; offset < missingModels.length; offset += 32) {
      if (controller.current.signal.aborted) return
      proposals.push(
        ...(await Promise.all(
          missingModels
            .slice(offset, offset + 32)
            .map((m) => batchQuoteRow(source, m))
        ))
      )
    }
    if (controller.current.signal.aborted) return
    setCatalog(source)
    setRows(proposals)
    setInitialized(true)
  }
  return (
    <section
      className='space-y-4'
      aria-label={t('Sync missing prices from models.dev')}
    >
      <p className='text-sm'>
        {t('Missing official prices')}: {missingModels.length} ·{' '}
        {t('Existing prices preserved')}: {models.length - missingModels.length}
      </p>
      {!initialized && (
        <Button
          type='button'
          disabled={busy || !missingModels.length}
          onClick={() => void run(load)}
        >
          {t('Sync missing prices from models.dev')}
        </Button>
      )}
      {error && (
        <p role='alert' className='text-destructive text-sm'>
          {t(error)}
        </p>
      )}
      {rows.slice(page * 20, page * 20 + 20).map((row) => (
        <div key={row.model} className='space-y-2 border-b py-3 text-sm'>
          <label className='flex items-center gap-2 font-medium'>
            <input
              type='checkbox'
              checked={row.selected}
              disabled={busy || published || !row.toml}
              onChange={(e) =>
                change(row.model, { selected: e.target.checked })
              }
            />
            {row.model}
          </label>
          {row.toml && (
            <p className='text-muted-foreground'>
              {t('Source')}: models.dev / {row.provider} / {row.sourceModel}
            </p>
          )}
          {!row.toml && (
            <div className='flex flex-wrap items-end gap-2'>
              <label>
                {t('Source model')}
                <Input
                  value={row.sourceModel}
                  disabled={busy}
                  onChange={(e) =>
                    change(row.model, {
                      sourceModel: e.target.value,
                      provider: '',
                      providers: [],
                    })
                  }
                />
              </label>
              {row.providers.length > 0 && (
                <label>
                  {t('Provider')}
                  <select
                    className='block rounded border p-2'
                    disabled={busy}
                    value={row.provider}
                    onChange={(e) =>
                      change(row.model, { provider: e.target.value })
                    }
                  >
                    <option value=''>{t('Select provider')}</option>
                    {row.providers.map((id) => (
                      <option key={id} value={id}>
                        {id}
                      </option>
                    ))}
                  </select>
                </label>
              )}
              <Button
                type='button'
                variant='outline'
                disabled={busy || !row.sourceModel.trim()}
                onClick={() =>
                  void run(async () =>
                    change(
                      row.model,
                      await batchQuoteRow(
                        catalog,
                        row.model,
                        row.sourceModel.trim(),
                        row.provider
                      )
                    )
                  )
                }
              >
                {t('Fetch upstream prices')}
              </Button>
            </div>
          )}
          {rowIssues.get(row.model)?.map((item) => (
            <div key={item} className='text-muted-foreground break-all'>
              {t(item)}
            </div>
          ))}
          {row.toml && (
            <details>
              <summary>{t('Review price fields')}</summary>
              <div className='grid gap-3 py-2 sm:grid-cols-2'>
                {(
                  [
                    'input',
                    'output',
                    'cache_read',
                    'cache_write',
                    'request',
                  ] as const
                ).map((meter) => {
                  let price: ReturnType<typeof readOfficialToml>
                  try {
                    price = readOfficialToml(row.model, row.toml)
                  } catch {
                    return null
                  }
                  if (price.tiered && meter !== 'request') return null
                  return (
                    <label key={meter}>
                      {meter} (
                      {meter === 'request' ? 'µUSD/request' : 'µUSD/1M tokens'})
                      <Input
                        type='text'
                        inputMode='numeric'
                        disabled={busy || published}
                        defaultValue={price.rates[meter] ?? ''}
                        key={String(price.rates[meter])}
                        onBlur={(e) => {
                          const input = e.target.value.trim()
                          if (
                            !input ||
                            !/^\d+$/.test(input) ||
                            !Number.isSafeInteger(Number(input))
                          ) {
                            if (input) setError(t('Invalid decimal price'))
                            e.target.value = String(price.rates[meter] ?? '')
                            return
                          }
                          if (price.rates[meter] === Number(input)) return
                          change(row.model, {
                            toml: writeOfficialToml(row.model, {
                              ...price,
                              source:
                                'manual (' +
                                row.provider +
                                ' / ' +
                                row.sourceModel +
                                ')',
                              rates: { ...price.rates, [meter]: Number(input) },
                            }),
                          })
                        }}
                      />
                    </label>
                  )
                })}
              </div>
              <details>
                <summary>{t('Advanced TOML')}</summary>
                <Textarea
                  value={row.toml}
                  disabled={busy || published}
                  onChange={(e) => change(row.model, { toml: e.target.value })}
                />
              </details>
            </details>
          )}
        </div>
      ))}
      {rows.length > 20 && (
        <div className='flex items-center gap-3'>
          <Button
            type='button'
            variant='ghost'
            disabled={page === 0}
            onClick={() => setPage(page - 1)}
          >
            {t('Previous')}
          </Button>
          <span>
            {page + 1} / {Math.ceil(rows.length / 20)}
          </span>
          <Button
            type='button'
            variant='ghost'
            disabled={(page + 1) * 20 >= rows.length}
            onClick={() => setPage(page + 1)}
          >
            {t('Next')}
          </Button>
        </div>
      )}
      {!!rows.length && !preview && !published && (
        <Button
          type='button'
          disabled={
            busy ||
            !selected.length ||
            selected.some((row) => rowIssues.get(row.model)?.length)
          }
          onClick={() =>
            void run(async () => {
              setPreview(
                await previewMissingOfficialPrices(
                  scope,
                  configuration,
                  new Map(selected.map((r) => [r.model, r.toml])),
                  channelId
                )
              )
            })
          }
        >
          {t('Preview selected prices')} ({selected.length})
        </Button>
      )}
      {preview && !published && (
        <div className='space-y-2'>
          <p role='status'>
            {t('Official base price')} · v
            {preview.body.expected_version.pricing} → v
            {preview.body.prices.version} · {preview.changes.added.join(', ')}
          </p>
          <Button
            type='button'
            disabled={busy}
            onClick={() =>
              void run(async () => {
                await publishOfficialPrice(scope, preview)
                setPublished(true)
                setPreview(undefined)
                await onPublished()
              })
            }
          >
            {t('Publish selected prices')}
          </Button>
        </div>
      )}
      {published && (
        <p role='status'>
          {t(
            'Official prices published. Review group routes before activation.'
          )}
        </p>
      )}
      {onClose && (
        <Button type='button' variant='ghost' disabled={busy} onClick={onClose}>
          {t('Close')}
        </Button>
      )}
    </section>
  )
}
