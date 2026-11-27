/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

import {
  NATIVE_ROOT,
  nativeRequest,
  type NativeConfiguration,
  type NativeVersion,
} from '@/features/channels/native-api'

import type { TerminalOperationContext } from './operations'

type CatalogPlan = {
  channels: number[]
  models: Map<number, string[]>
  version: NativeVersion
  reason: 'catalog-refresh' | 'operator-review'
}
export type CatalogContext = TerminalOperationContext & {
  catalogs: Map<string, CatalogPlan>
}
const modelsSchema = z.array(z.string().min(1)).max(10000)
function read(context: CatalogContext) {
  return nativeRequest<NativeConfiguration>(
    context.scope,
    'GET',
    NATIVE_ROOT + '/configuration',
    undefined,
    { signal: context.signal }
  )
}
export function catalogCoverage(
  config: NativeConfiguration,
  channel: number,
  models: string[]
) {
  const routes = config.routing.routes.filter((r) =>
    r.targets.some((t) => t.upstream_id === channel)
  )
  const attached = new Set(
    routes.flatMap((r) =>
      r.targets
        .filter((t) => t.upstream_id === channel)
        .map((t) => t.upstream_model)
    )
  )
  const previous = config.bindings.discoveries[channel]?.models ?? []
  const observed = new Set(models)
  const activePrices = config.prices.rules
  const official = config.bindings.official_prices?.models ?? {}
  const missing = models.filter((m) => {
    const hasBase = Object.hasOwn(official, m)
    const mapped = routes.filter((r) =>
      r.targets.some((t) => t.upstream_id === channel && t.upstream_model === m)
    )
    if (!mapped.length)
      return !hasBase && !activePrices.some((p) => p.model === m)
    return mapped.some((r) =>
      r.speeds.some(
        (speed) =>
          !activePrices.some(
            (p) =>
              p.model === r.model && p.group === r.group && p.speed === speed
          )
      )
    )
  })
  const missing_price_routes = routes.flatMap((r) =>
    r.targets
      .filter(
        (t) => t.upstream_id === channel && missing.includes(t.upstream_model)
      )
      .map((t) => ({
        model: r.model,
        upstream_model: t.upstream_model,
        channel,
        group: r.group,
        route: r.id,
      }))
  )
  return {
    missing_price_routes,
    unbound: models.filter((m) => !attached.has(m)),
    newly_discovered: models.filter(
      (m) => !previous.includes(m) && !attached.has(m)
    ),
    onboarded: [...attached].filter((m) => observed.has(m)),
    no_longer_offered: [...new Set([...previous, ...attached])].filter(
      (m) => !observed.has(m)
    ),
    missing_price: missing,
  }
}
export async function previewCatalog(
  channels: number[],
  reason: CatalogPlan['reason'],
  context: CatalogContext
) {
  const config = await read(context)
  const models = new Map<number, string[]>()
  const rows = []
  for (const channel of channels) {
    context.signal?.throwIfAborted()
    if (!config.routing.upstreams.some((u) => u.id === channel)) {
      rows.push({ channel, status: 'blocked', reason: 'channel_not_found' })
      continue
    }
    try {
      const items = modelsSchema.parse(
        await nativeRequest(
          context.scope,
          'GET',
          '/api/channel/fetch_models/' + channel,
          undefined,
          { signal: context.signal }
        )
      )
      models.set(channel, items)
      rows.push({
        channel,
        status: 'ready',
        ...catalogCoverage(config, channel, items),
      })
    } catch (e) {
      context.signal?.throwIfAborted()
      rows.push({ channel, status: 'blocked', reason: terminalError(e) })
    }
  }
  const preview = crypto.randomUUID()
  if (models.size) {
    if (context.catalogs.size >= 16)
      context.catalogs.delete(context.catalogs.keys().next().value!)
    context.catalogs.set(preview, {
      channels,
      models,
      version: config.version,
      reason,
    })
  }
  return {
    status: 'preview',
    preview: models.size ? preview : null,
    expected_version: config.version,
    rows,
  }
}
export async function confirmCatalog(
  preview: string,
  channels: number[],
  context: CatalogContext
) {
  const plan = context.catalogs.get(preview)
  if (!plan || channels.some((id) => !plan.models.has(id)))
    throw new Error('preview_missing_or_unapproved_channel')
  context.catalogs.delete(preview)
  let version = plan.version
  const applied: number[] = []
  for (const channel of channels) {
    try {
      context.signal?.throwIfAborted()
      const result = await nativeRequest<{ version: NativeVersion }>(
        context.scope,
        'POST',
        NATIVE_ROOT + '/channels/' + channel + '/models/synchronize',
        {
          expected_version: version,
          expected_models: plan.models.get(channel),
          reason: plan.reason,
        },
        { signal: context.signal }
      )
      version = result.version
      applied.push(channel)
    } catch (e) {
      const remaining = channels.slice(applied.length)
      if ((e as { response?: { status?: number } })?.response?.status === 409) {
        return {
          status: 'conflict',
          applied,
          channel,
          version,
          replacement: await previewCatalog(remaining, plan.reason, context),
        }
      }
      return {
        status: 'stopped',
        applied,
        channel,
        version,
        reason: terminalError(e),
        remaining: remaining.slice(1),
      }
    }
  }
  const active = await read(context)
  return {
    status: 'published',
    applied,
    version,
    active_version: active.version,
  }
}
export function terminalError(e: unknown): string {
  if (e instanceof z.ZodError)
    return e.issues
      .map((i) => `${i.path.join('.') || 'arguments'}: ${i.code}`)
      .join(', ')
  const http = e as {
    response?: { status?: number; data?: { code?: unknown } }
  }
  if (http?.response?.status) {
    const code = http.response.data?.code
    return (
      'HTTP ' +
      http.response.status +
      (typeof code === 'string' && /^[A-Z0-9_]{1,100}$/.test(code)
        ? ' ' + code
        : '')
    )
  }
  if (e instanceof Error && e.name === 'AbortError') return 'cancelled'
  return e instanceof Error && /^[\w .:/,()[\]-]{1,200}$/.test(e.message)
    ? e.message
    : 'operation_failed'
}
