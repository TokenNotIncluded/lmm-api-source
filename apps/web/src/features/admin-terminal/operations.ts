/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

import {
  presetProtocolEndpoint,
  resolveNativeProvider,
} from '@/features/channels/lib/native-provider'
import { nativeProtocols } from '@/features/channels/lib/native-routing-form'
import {
  NATIVE_ROOT,
  nativeRequest,
  type NativeConfiguration,
  type NativeRoutingScope,
  type NativeVersion,
} from '@/features/channels/native-api'
import {
  fractionText,
  samePriceRule,
} from '@/features/system-settings/billing/native-price-rules'
import {
  cacheWriteCoverageError,
  cacheWriteTariffTarget,
  officialPriceDestinations,
} from '@/features/system-settings/billing/official-cache-write-coverage'
import {
  fetchModelsDevCatalog,
  officialModelsDevQuote,
} from '@/features/system-settings/billing/official-models-dev-quote'
import { proposedPublication } from '@/features/system-settings/billing/official-price-api'
import {
  mergeQuote,
  missingOfficialRatePaths,
  readOfficialToml,
  writeOfficialToml,
  type OfficialPrice,
} from '@/features/system-settings/billing/official-price-toml'

const configuration = `${NATIVE_ROOT}/configuration`
const name = z
  .string()
  .min(1)
  .max(200)
  .regex(/^[^\p{Cc}]+$/u)
const reason = z.enum([
  'catalog-refresh',
  'missing-price',
  'protocol-correction',
  'operator-review',
])
const names = z
  .array(name)
  .min(1)
  .max(100)
  .refine((list) => new Set(list).size === list.length)
const prices = z
  .object({
    provider: name,
    models: names,
    request_rate: z.literal(0).optional(),
    reason: reason.default('missing-price'),
  })
  .strict()
const protocols = z
  .object({
    from: z.enum(nativeProtocols),
    to: z.enum(nativeProtocols),
    channels: z
      .array(z.number().int().positive().safe())
      .min(1)
      .max(100)
      .optional(),
    reason: reason.default('protocol-correction'),
  })
  .strict()
const confirmation = z
  .object({ preview: z.string().uuid(), items: names })
  .strict()

type PriceCommand = z.infer<typeof prices>
type ProtocolCommand = z.infer<typeof protocols>
type Change =
  | { kind: 'price'; id: string; toml: string }
  | {
      kind: 'protocol'
      id: string
      channel: number
      protocol: string
      endpoint: string
    }
type Publication = Pick<
  NativeConfiguration,
  'routing' | 'prices' | 'bindings'
> & { expected_version: NativeVersion; reason: z.infer<typeof reason> }
export type TerminalPreview = {
  actor: NativeRoutingScope
  version: NativeVersion
  command: 'prices.preview' | 'protocols.preview'
  args: PriceCommand | ProtocolCommand
  changes: Change[]
  digest: string
}
export type TerminalOperationContext = {
  scope: NativeRoutingScope
  previews: Map<string, TerminalPreview>
  signal?: AbortSignal
}
export type TerminalOperationRow = {
  id: string
  status: 'ready' | 'blocked' | 'preserved'
  details?: unknown
  reason?: string
}

function sameVersion(a: NativeVersion | undefined, b: NativeVersion) {
  return a?.routing === b.routing && a?.pricing === b.pricing
}
function sameActor(a: NativeRoutingScope, b: NativeRoutingScope) {
  return (
    a.userId === b.userId && a.role === b.role && a.sessionId === b.sessionId
  )
}
function readConfig(context: TerminalOperationContext) {
  return nativeRequest<NativeConfiguration>(
    context.scope,
    'GET',
    configuration,
    undefined,
    { signal: context.signal }
  )
}
function serverFailure(error: unknown) {
  const http = error as {
    isAxiosError?: boolean
    response?: { status?: number; data?: { code?: string; message?: string } }
  }
  return http?.isAxiosError ? http.response : undefined
}
function failure(error: unknown) {
  const response = serverFailure(error)
  if (
    response?.data?.code?.startsWith('CHANNEL_') &&
    typeof response.data.message === 'string'
  ) {
    return response.data.message
  }
  return error instanceof Error
    ? error.message
    : 'configuration_validation_failed'
}

async function priceChange(
  config: NativeConfiguration,
  catalog: unknown,
  id: string,
  args: PriceCommand
): Promise<Change> {
  const bound = config.routing.routes.filter((row) => row.model === id)
  const upstreamIds = new Set(
    bound.flatMap((row) => row.targets.map((target) => target.upstream_model))
  )
  if (upstreamIds.size > 1) throw new Error('ambiguous_upstream_model_ids')
  if (!bound.length) {
    const publicModels = [
      ...new Set(
        config.routing.routes
          .filter((row) =>
            row.targets.some((target) => target.upstream_model === id)
          )
          .map((row) => row.model)
      ),
    ]
    if (publicModels.length)
      throw new Error('use_public_model_ids: ' + publicModels.join(', '))
  }
  const sourceModel = [...upstreamIds][0] ?? id
  const fetched = officialModelsDevQuote(catalog, sourceModel, args.provider)
  if (!fetched) throw new Error('exact_provider_model_price_missing')
  const quote = {
    ...fetched,
    model: id,
    source: {
      kind: 'models_dev' as const,
      provider: args.provider,
      ...(sourceModel === id ? {} : { source_model: sourceModel }),
    },
  }
  // A missing request fee must be confirmed separately on token-based imports.
  const input = writeOfficialToml(id, {
    source: 'manual',
    source_unit: 'micro-USD/1M tokens',
    rates: args.request_rate === 0 ? { request: 0 } : {},
  })
  const toml = await mergeQuote(id, input, quote)
  const price = readOfficialToml(id, toml)
  const destinations = officialPriceDestinations(config.routing, id)
  for (const [channel, discovery] of Object.entries(
    config.bindings.discoveries
  )) {
    if (!discovery.pending_models?.includes(id)) continue
    const upstream = config.routing.upstreams.find(
      (row) => String(row.id) === channel
    )
    if (upstream) destinations.push({ protocol: upstream.protocol })
  }
  const missing = missingOfficialRatePaths(
    price,
    cacheWriteTariffTarget(price, destinations)
  )
  if (missing.length) throw new Error(missing.join(', '))
  if (price.unmapped_rates)
    throw new Error(
      Object.keys(price.unmapped_rates)
        .map((key) => 'unmapped_rates.' + key)
        .join(', ')
    )
  const issue = cacheWriteCoverageError(price, destinations)
  if (issue) throw new Error(issue)
  return { kind: 'price', id, toml }
}

function protocolChange(
  config: NativeConfiguration,
  id: number,
  args: ProtocolCommand
): Change {
  const row = config.routing.upstreams.find((channel) => channel.id === id)
  if (!row) throw new Error('channel_not_found')
  if (row.protocol !== args.from) throw new Error('source_protocol_mismatch')
  if (args.from === args.to) throw new Error('source_and_target_protocol_match')
  const preset = resolveNativeProvider(
    config,
    config.bindings.channel_providers?.[row.id],
    row.protocol,
    row.endpoint
  )
  const supplied = presetProtocolEndpoint(
    preset,
    row.protocol,
    row.endpoint,
    args.to
  )
  if (
    preset &&
    preset.id !== 'custom' &&
    preset.protocol_endpoints?.length &&
    ![preset, ...preset.protocol_endpoints].some(
      (variant) => variant.protocol === args.to
    )
  ) {
    throw new Error('supplier_preset_does_not_offer_target_protocol')
  }
  const endpoint = supplied ?? row.endpoint
  if (new URL(endpoint).origin !== new URL(row.endpoint).origin)
    throw new Error('destination_origin_change_requires_channel_review')
  return {
    kind: 'protocol',
    id: String(id),
    channel: id,
    protocol: args.to,
    endpoint,
  }
}

function publication(
  config: NativeConfiguration,
  changes: Change[],
  why: Publication['reason']
): Publication {
  let bindings = config.bindings
  let prices = config.prices
  const channels = new Map<number, Extract<Change, { kind: 'protocol' }>>()
  for (const change of changes) {
    if (change.kind === 'price') {
      const next = proposedPublication(
        { ...config, bindings },
        change.id,
        change.toml
      )
      bindings = next.bindings
      prices = next.prices
    } else channels.set(change.channel, change)
  }
  const routing = {
    ...config.routing,
    version: config.version.routing + 1,
    upstreams: config.routing.upstreams.map((row) => {
      const next = channels.get(row.id)
      return next
        ? { ...row, protocol: next.protocol, endpoint: next.endpoint }
        : row
    }),
  }
  if (channels.size)
    bindings = {
      ...bindings,
      contexts: bindings.contexts.map((row) => {
        const next = channels.get(row.upstream_id)
        return next
          ? { ...row, protocol: next.protocol, endpoint: next.endpoint }
          : row
      }),
    }
  return {
    expected_version: config.version,
    routing,
    prices,
    bindings,
    reason: why,
  }
}
type ServerPreview = Pick<
  Publication,
  'bindings' | 'routing' | 'prices' | 'expected_version'
> & { saved: false; confirmation_digest: string }
async function previewBody(
  body: Publication,
  context: TerminalOperationContext
) {
  const response = await nativeRequest<ServerPreview>(
    context.scope,
    'POST',
    configuration + '/preview',
    body,
    { signal: context.signal }
  )
  if (
    response?.saved !== false ||
    !sameVersion(response.expected_version, body.expected_version) ||
    !/^[a-f0-9]{64}$/.test(response.confirmation_digest) ||
    !Array.isArray(response.routing?.routes) ||
    !Array.isArray(response.prices?.rules)
  ) {
    throw new Error('invalid_configuration_preview')
  }
  return response
}
function effectiveRates(price: OfficialPrice, group: number, speed: number) {
  const scale = BigInt(group) * BigInt(speed)
  const scaled = (n: unknown, d: unknown) =>
    fractionText({
      numerator: String(BigInt(String(n)) * scale),
      denominator: String(BigInt(String(d)) * 1_000_000_000_000n),
    })
  if (price.tiered)
    return price.tiered.tiers.map((t) => ({
      up_to_input_tokens: t.up_to_input_tokens ?? null,
      rates: Object.fromEntries(
        Object.entries(t.rates).map(([meter, rate]) => [
          meter,
          scaled(rate.numerator, rate.denominator),
        ])
      ),
    }))
  return Object.fromEntries(
    Object.entries(price.rates).map(([meter, rate]) => [
      meter,
      scaled(rate, 1_000_000),
    ])
  )
}
function changeDetails(
  config: NativeConfiguration,
  server: ServerPreview,
  change: Change
) {
  if (change.kind === 'protocol') {
    const routes = config.routing.routes.filter((r) =>
      r.targets.some((t) => t.upstream_id === change.channel)
    )
    return {
      channel_id: change.channel,
      from: config.routing.upstreams.find((row) => row.id === change.channel)
        ?.protocol,
      to: change.protocol,
      routes: routes.map((r) => ({ id: r.id, group: r.group, model: r.model })),
      upstream_models: [
        ...new Set(
          routes.flatMap((r) =>
            r.targets
              .filter((t) => t.upstream_id === change.channel)
              .map((t) => t.upstream_model)
          )
        ),
      ],
      pending_before:
        config.bindings.discoveries[change.channel]?.pending_models ?? [],
      pending_after:
        server.bindings.discoveries[change.channel]?.pending_models ?? [],
    }
  }
  const official = server.bindings.official_prices
  if (!official || !Object.hasOwn(official.models, change.id))
    throw new Error('invalid_price_preview')
  const price = official.models[change.id]
  const effective = server.prices.rules
    .filter((rule) => rule.model === change.id)
    .map((rule) => {
      const group = Number(rule.group_multiplier_ppm),
        speed = Number(rule.speed_multiplier_ppm)
      if (!Number.isSafeInteger(group) || !Number.isSafeInteger(speed))
        throw new Error('invalid_multipliers')
      return {
        group: rule.group,
        speed: rule.speed,
        group_multiplier_ppm: group,
        speed_multiplier_ppm: speed,
        final: effectiveRates(price, group, speed),
      }
    })
  // Discovery alone does not publish a group route. Show projections for
  // declared groups separately from the executable rules compiled by Core.
  const declared = new Set<string>()
  for (const [channel, row] of Object.entries(config.bindings.discoveries)) {
    if (
      !row.models.includes(change.id) &&
      !row.pending_models?.includes(change.id)
    )
      continue
    for (const group of config.bindings.channel_groups?.[channel] ?? [])
      declared.add(group)
  }
  const planned = effective.length
    ? []
    : [...declared].sort().map((group) => {
        const multiplier = official.group_multipliers_ppm[group] ?? 1_000_000
        return {
          group,
          speed: 'standard',
          group_multiplier_ppm: multiplier,
          speed_multiplier_ppm: 1_000_000,
          final: effectiveRates(price, multiplier, 1_000_000),
        }
      })
  return {
    model: change.id,
    pricing_scope: effective.length ? 'route_rules' : 'official_base_only',
    declared_group_projection: planned,
    source: price.source,
    unit: 'USD/1M tokens; request: USD/request',
    base: effectiveRates(price, 1_000_000, 1_000_000),
    effective,
    routes: server.routing.routes
      .filter((r) => r.model === change.id)
      .map((r) => ({ id: r.id, group: r.group })),
  }
}

async function prepare(
  command: TerminalPreview['command'],
  raw: Record<string, unknown>,
  context: TerminalOperationContext
) {
  const config = await readConfig(context)
  const rows: TerminalOperationRow[] = []
  const changes: Change[] = []
  const args =
    command === 'prices.preview' ? prices.parse(raw) : protocols.parse(raw)
  let digest = ''
  async function validate(id: string, build: () => Change | Promise<Change>) {
    context.signal?.throwIfAborted()
    try {
      const change = await build()
      const response = await previewBody(
        publication(config, [change], args.reason),
        context
      )
      const details = changeDetails(config, response, change)
      digest = response.confirmation_digest
      changes.push(change)
      rows.push({ id, status: 'ready', details })
    } catch (error) {
      context.signal?.throwIfAborted()
      // Transport failures and permission loss stop the command, not individual rows.
      const response = serverFailure(error)
      const rowConflict = [
        'CHANNEL_BINDING_NOT_CONFIGURED',
        'CHANNEL_TARGET_TIMEOUT_TOO_SHORT',
      ].includes(response?.data?.code ?? '')
      if (
        (error as { isAxiosError?: boolean })?.isAxiosError &&
        response?.status !== 400 &&
        response?.status !== 422 &&
        !rowConflict
      )
        throw error
      rows.push({ id, status: 'blocked', reason: failure(error) })
    }
  }
  if ('models' in args) {
    const catalog = await fetchModelsDevCatalog(context.signal)
    for (const id of args.models) {
      if (
        Object.hasOwn(config.bindings.official_prices?.models ?? {}, id) ||
        config.prices.rules.some((r) => r.model === id)
      ) {
        rows.push({ id, status: 'preserved', reason: 'existing_price' })
        continue
      }
      const known =
        config.routing.routes.some((r) => r.model === id) ||
        Object.values(config.bindings.discoveries).some(
          (d) => d.models.includes(id) || d.pending_models?.includes(id)
        )
      if (!known) {
        rows.push({
          id,
          status: 'blocked',
          reason: 'model_not_in_routing_or_discovery',
        })
        continue
      }
      await validate(id, () => priceChange(config, catalog, id, args))
    }
  } else {
    const ids =
      args.channels ??
      config.routing.upstreams
        .filter((u) => u.protocol === args.from)
        .map((u) => u.id)
    if (new Set(ids).size !== ids.length) throw new Error('duplicate_channel')
    for (const id of ids)
      await validate(String(id), () => protocolChange(config, id, args))
  }
  if (changes.length > 1) {
    const combined = await previewBody(
      publication(config, changes, args.reason),
      context
    )
    digest = combined.confirmation_digest
    for (const row of rows) {
      const change = changes.find((c) => c.id === row.id)
      if (change) row.details = changeDetails(config, combined, change)
    }
  }
  const preview = changes.length ? crypto.randomUUID() : null
  if (preview) {
    if (context.previews.size >= 32) {
      const oldest = context.previews.keys().next().value
      if (oldest) context.previews.delete(oldest)
    }
    context.previews.set(preview, {
      actor: context.scope,
      version: config.version,
      command,
      args,
      changes,
      digest,
    })
  }
  return {
    status: 'preview' as const,
    preview,
    expected_version: config.version,
    reason: args.reason,
    rows,
  }
}

export async function executeTerminalOperation(
  command: string,
  raw: Record<string, unknown>,
  context: TerminalOperationContext
) {
  if (command === 'prices.preview' || command === 'protocols.preview')
    return prepare(command, raw, context)
  if (command !== 'changes.confirm')
    throw new Error('unknown_terminal_operation')
  const args = confirmation.parse(raw),
    plan = context.previews.get(args.preview)
  if (!plan || !sameActor(plan.actor, context.scope))
    throw new Error('preview_missing_or_expired')
  const selected = plan.changes.filter((change) =>
    args.items.includes(change.id)
  )
  if (selected.length !== args.items.length)
    throw new Error('selection_contains_unapproved_item')
  // Consume before the first await: a duplicated click cannot dispatch another write.
  context.previews.delete(args.preview)
  const refresh = async () => ({
    status: 'conflict',
    replacement: await prepare(plan.command, plan.args, context),
  })
  const config = await readConfig(context)
  if (!sameVersion(config.version, plan.version)) return refresh()
  const body = publication(config, selected, plan.args.reason)
  let preview: ServerPreview
  try {
    preview = await previewBody(body, context)
    if (
      selected.length === plan.changes.length &&
      preview.confirmation_digest !== plan.digest
    )
      return refresh()
    const receipt = await nativeRequest<{ version: NativeVersion }>(
      context.scope,
      'PUT',
      configuration,
      { ...body, confirmation_digest: preview.confirmation_digest },
      { signal: context.signal }
    )
    const actual = await readConfig(context)
    const current = sameVersion(receipt.version, actual.version)
    if (current)
      for (const change of selected) {
        const matches =
          change.kind === 'price'
            ? samePriceRule(
                actual.bindings.official_prices?.models[change.id],
                preview.bindings.official_prices?.models[change.id]
              )
            : actual.routing.upstreams.some(
                (row) =>
                  row.id === change.channel &&
                  row.protocol === change.protocol &&
                  row.endpoint === change.endpoint
              )
        if (!matches)
          throw new Error('published_configuration_readback_mismatch')
      }
    return {
      status: current ? 'published' : 'published_version_advanced',
      readback: current
        ? selected.map((change) => ({
            id: change.id,
            scope:
              change.kind === 'protocol'
                ? 'channel_protocol'
                : actual.prices.rules.some((rule) => rule.model === change.id)
                  ? 'route_rules'
                  : 'official_base_only',
          }))
        : [],
      selected: args.items,
      reason: plan.args.reason,
      version: receipt.version,
      active_version: actual.version,
      confirmed_digest: preview.confirmation_digest,
    }
  } catch (error) {
    if (serverFailure(error)?.data?.code === 'CHANNEL_VERSION_CONFLICT')
      return refresh()
    throw error
  }
}
