/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import {
  NATIVE_ROOT,
  nativeRequest,
  type NativeConfiguration,
  type NativeRoutingScope,
  type NativeVersion,
} from '@/features/channels/native-api'

import { samePriceRule } from './native-price-rules'
import {
  cacheWriteCoverageError,
  officialPriceDestinations,
} from './official-cache-write-coverage'
import {
  fetchOfficialModelsDevQuote,
  type OfficialSourceQuote,
} from './official-models-dev-quote'
import {
  completeOfficialPrice,
  mergeQuote,
  readOfficialToml,
  writeOfficialToml,
  type OfficialCatalog,
} from './official-price-toml'

const configPath = `${NATIVE_ROOT}/configuration`
function catalogOf(config: NativeConfiguration) {
  return config.bindings.official_prices
}
function initializeCatalog(config: NativeConfiguration): OfficialCatalog {
  const groups = new Map<string, number>(),
    speeds = new Map<string, number>()
  const remember = (
    values: Map<string, number>,
    key: unknown,
    ppm: unknown
  ) => {
    if (
      typeof key !== 'string' ||
      typeof ppm !== 'number' ||
      !Number.isSafeInteger(ppm) ||
      (values.has(key) && values.get(key) !== ppm)
    ) {
      throw new Error('Existing price multipliers require review')
    }
    values.set(key, ppm)
  }
  for (const rule of config.prices.rules) {
    remember(groups, rule.group, rule.group_multiplier_ppm)
    remember(speeds, rule.speed, rule.speed_multiplier_ppm)
  }
  if (speeds.has('standard') && speeds.get('standard') !== 1_000_000) {
    throw new Error('Existing standard multiplier requires review')
  }
  return {
    models: {},
    group_multipliers_ppm: Object.fromEntries(
      config.routing.groups.map(({ name }) => [
        name,
        groups.get(name) ?? 1_000_000,
      ])
    ),
    speed_multipliers_ppm: {
      fast: speeds.get('fast') ?? 1_000_000,
      ultrafast: speeds.get('ultrafast') ?? 1_000_000,
    },
  }
}
function own<T>(object: Record<string, T>, id: string): T | undefined {
  return Object.hasOwn(object, id) ? object[id] : undefined
}

export async function readOfficialPriceToml(
  _scope: NativeRoutingScope,
  configuration: NativeConfiguration,
  model: string
) {
  return writeOfficialToml(
    model,
    own(catalogOf(configuration)?.models ?? {}, model)
  )
}
type Source =
  | { kind: 'models.dev'; provider: string; sourceModel?: string }
  | {
      kind: 'channel'
      channelId: number
      upstreamModel?: string
      format?: 'openrouter_usd_per_token'
    }
export async function syncOfficialPriceToml(
  scope: NativeRoutingScope,
  model: string,
  toml: string,
  source: Source
) {
  let quote: OfficialSourceQuote | null
  if (source.kind === 'models.dev') {
    const sourceModel = source.sourceModel || model
    const fetched = await fetchOfficialModelsDevQuote(
      sourceModel,
      source.provider
    )
    quote = fetched && {
      ...fetched,
      model,
      source: {
        kind: 'models_dev',
        provider: source.provider,
        ...(sourceModel !== model ? { source_model: sourceModel } : {}),
      },
    }
  } else {
    const observed = await nativeRequest<{
      channel_id: number
      model: string
      quote: OfficialSourceQuote | null
    }>(
      scope,
      'POST',
      `${NATIVE_ROOT}/channels/${source.channelId}/models/prices/preview`,
      {
        model,
        ...(source.upstreamModel
          ? { upstream_model: source.upstreamModel }
          : {}),
        ...(source.format ? { format: source.format } : {}),
      }
    )
    quote = observed.quote
    if (
      observed.model !== model ||
      observed.channel_id !== source.channelId ||
      (quote !== null &&
        (quote.source.kind !== 'channel' ||
          quote.source.channel_id !== source.channelId ||
          (source.upstreamModel != null &&
            quote.source.upstream_model !== source.upstreamModel)))
    ) {
      throw new Error('Invalid channel price response')
    }
  }
  return quote
    ? { toml: await mergeQuote(model, toml, quote), found: true }
    : { toml, found: false }
}
type Publication = {
  expected_version: NativeVersion
  routing: NativeConfiguration['routing']
  prices: NativeConfiguration['prices']
  bindings: NativeConfiguration['bindings'] & {
    official_prices: OfficialCatalog
  }
}
export function proposedPublication(
  config: NativeConfiguration,
  model: string,
  toml: string | null
): Publication {
  const catalog = catalogOf(config) ?? initializeCatalog(config)
  const models = { ...catalog.models }
  if (toml === null) {
    if (!Object.hasOwn(models, model)) {
      throw new Error('Model has no official price')
    }
    delete models[model]
  } else {
    Object.defineProperty(models, model, {
      value: completeOfficialPrice(readOfficialToml(model, toml)),
      writable: true,
      configurable: true,
      enumerable: true,
    })
  }
  return {
    expected_version: config.version,
    routing: { ...config.routing, version: config.version.routing + 1 },
    prices: { ...config.prices, version: config.version.pricing + 1 },
    bindings: { ...config.bindings, official_prices: { ...catalog, models } },
  }
}
export function officialModelChanges(
  before: OfficialCatalog['models'],
  after: OfficialCatalog['models']
) {
  const ids = Object.keys(after).sort()
  return {
    added: ids.filter((k) => !Object.hasOwn(before, k)),
    modified: ids.filter(
      (k) => Object.hasOwn(before, k) && !samePriceRule(before[k], after[k])
    ),
    deleted: Object.keys(before)
      .filter((k) => !Object.hasOwn(after, k))
      .sort(),
  }
}
export type OfficialPricePreview = {
  changes: ReturnType<typeof officialModelChanges>
  body: Publication
  digest: string
}
export async function previewOfficialPriceToml(
  scope: NativeRoutingScope,
  configuration: NativeConfiguration,
  model: string,
  toml: string | null,
  pendingChannel?: number,
  sourceChannel?: number
): Promise<OfficialPricePreview> {
  // The regular display projection may omit unrelated full-document bindings.
  // Use the authoritative revision without refreshing away a stale user's edits.
  const raw = await nativeRequest<NativeConfiguration>(scope, 'GET', configPath)
  if (!samePriceRule(raw.version, configuration.version)) {
    throw new Error('The routing snapshot changed. Reload before continuing.')
  }
  const body = proposedPublication(raw, model, toml)
  if (toml !== null) {
    const issue = cacheWriteCoverageError(
      body.bindings.official_prices.models[model],
      officialPriceDestinations(
        raw.routing,
        model,
        pendingChannel,
        sourceChannel
      )
    )
    if (issue) throw new Error(issue)
  }
  return validatePublication(scope, raw, body, false)
}

export async function previewMissingOfficialPrices(
  scope: NativeRoutingScope,
  configuration: NativeConfiguration,
  drafts: ReadonlyMap<string, string>,
  channelId?: number
): Promise<OfficialPricePreview> {
  if (!drafts.size) throw new Error('Select at least one model')
  const raw = await nativeRequest<NativeConfiguration>(scope, 'GET', configPath)
  if (!samePriceRule(raw.version, configuration.version)) {
    throw new Error('The routing snapshot changed. Reload before continuing.')
  }
  const catalog = catalogOf(raw) ?? initializeCatalog(raw)
  const models = { ...catalog.models }
  for (const [model, toml] of drafts) {
    if (Object.hasOwn(models, model))
      throw new Error('Existing official prices are preserved')
    const price = completeOfficialPrice(readOfficialToml(model, toml))
    const issue = cacheWriteCoverageError(
      price,
      officialPriceDestinations(raw.routing, model, channelId)
    )
    if (issue) throw new Error(issue)
    Object.defineProperty(models, model, {
      value: price,
      enumerable: true,
      writable: true,
      configurable: true,
    })
  }
  const body: Publication = {
    expected_version: raw.version,
    routing: { ...raw.routing, version: raw.version.routing + 1 },
    prices: { ...raw.prices, version: raw.version.pricing + 1 },
    bindings: { ...raw.bindings, official_prices: { ...catalog, models } },
  }
  const preview = await validatePublication(scope, raw, body, true)
  if (
    preview.changes.modified.length ||
    preview.changes.deleted.length ||
    !samePriceRule(preview.changes.added, [...drafts.keys()].sort())
  ) {
    throw new Error('Invalid price publication preview')
  }
  return preview
}

async function validatePublication(
  scope: NativeRoutingScope,
  raw: NativeConfiguration,
  body: Publication,
  preserveRules: boolean
): Promise<OfficialPricePreview> {
  const preview = await nativeRequest<{
    saved: boolean
    expected_version: NativeVersion
    confirmation_digest: string
    bindings: Publication['bindings']
    prices?: NativeConfiguration['prices']
  }>(scope, 'POST', `${configPath}/preview`, body)
  if (
    preview.saved !== false ||
    !samePriceRule(preview.expected_version, body.expected_version) ||
    !/^[0-9a-f]{64}$/.test(preview.confirmation_digest) ||
    !preview.bindings?.official_prices?.models
  ) {
    throw new Error('Invalid price publication preview')
  }
  if (preserveRules) {
    if (!Array.isArray(preview.prices?.rules))
      throw new Error('Invalid price publication preview')
    const byId = new Map(preview.prices.rules.map((rule) => [rule.id, rule]))
    for (const previous of raw.prices.rules) {
      if (!samePriceRule(previous, byId.get(previous.id))) {
        throw new Error(
          'An existing group price would change. Review it individually.'
        )
      }
    }
  }
  return {
    body,
    digest: preview.confirmation_digest,
    changes: officialModelChanges(
      catalogOf(raw)?.models ?? {},
      preview.bindings.official_prices.models
    ),
  }
}
export async function publishOfficialPrice(
  scope: NativeRoutingScope,
  preview: OfficialPricePreview
) {
  return nativeRequest(scope, 'PUT', configPath, {
    ...preview.body,
    confirmation_digest: preview.digest,
  })
}
