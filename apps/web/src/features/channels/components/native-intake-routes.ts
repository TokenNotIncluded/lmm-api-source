/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import {
  nativePriceRuleSchema,
  samePriceRule,
} from '@/features/system-settings/billing/native-price-rules'
import { cacheWriteCoverageError } from '@/features/system-settings/billing/official-cache-write-coverage'

import { newNativeContext } from '../lib/native-routing-form'
import { newNativeTargetTimeoutMs } from '../lib/native-target-timeout'
import {
  NATIVE_ROOT,
  nativeRequest,
  type NativeConfiguration,
  type NativeRoutingScope,
  type NativeUpstream,
  type NativeModelRoute,
  type NativeContext,
} from '../native-api'
import { defaultModelDisplayName } from './native-model-draft'

export type IntakeModel = { upstream: string; model: string }
export type IntakeLimits = { input: string; output: string; revision: string }
export const emptyLimits: IntakeLimits = { input: '', output: '', revision: '' }
const key = (model: unknown, group: unknown) => JSON.stringify([model, group])

export function intakeIndex(config: NativeConfiguration) {
  return {
    routes: new Map(
      config.routing.routes.map((r) => [key(r.model, r.group), r])
    ),
    rules: new Map(
      config.prices.rules.map((r) => [
        JSON.stringify([r.model, r.group, r.speed]),
        r,
      ])
    ),
    groups: new Set(
      config.routing.groups.filter((g) => g.enabled).map((g) => g.name)
    ),
    contexts: new Map(
      config.bindings.contexts.map((c) => [
        key(c.upstream_id, c.upstream_model),
        c,
      ])
    ),
    observed: new Map(
      Object.entries(config.bindings.discoveries).map(([id, discovery]) => [
        Number(id),
        new Set(discovery.models),
      ])
    ),
    upstreams: new Map(config.routing.upstreams.map((u) => [u.id, u])),
  }
}
export function inputRequirements(channel: NativeUpstream) {
  const tool = ['deepseek_search', 'deepseek_fetch'].includes(channel.protocol)
  return {
    input: !tool,
    output:
      !tool && !['openai_embeddings', 'jina_rerank'].includes(channel.protocol),
  }
}
function positiveLimit(value: string) {
  return /^[1-9]\d*$/.test(value) && Number.isSafeInteger(Number(value))
}
export function groupIntake(
  config: NativeConfiguration,
  index: ReturnType<typeof intakeIndex>,
  channel: NativeUpstream,
  pair: IntakeModel,
  group: string,
  limits: IntakeLimits = emptyLimits
) {
  const reasons: string[] = []
  if (!pair.model.trim()) reasons.push('Choose a public model ID.')
  if (!index.observed.get(channel.id)?.has(pair.upstream))
    reasons.push('Model absent from the latest upstream catalog.')
  if (!channel.enabled) reasons.push('Enable this channel first.')
  if (!index.groups.has(group)) reasons.push('Enable this routing group first.')
  const catalog = config.bindings.official_prices
  const base =
    catalog && Object.hasOwn(catalog.models, pair.model)
      ? catalog.models[pair.model]
      : undefined
  const route = index.routes.get(key(pair.model, group))
  const matches =
    route?.targets.filter((t) => t.upstream_id === channel.id) ?? []
  const target = matches[0]
  const speed = route?.speeds.includes('standard')
    ? 'standard'
    : (route?.speeds[0] ?? 'standard')
  const coveredSpeeds = target
    ? target.speeds.filter((s) => route?.speeds.includes(s))
    : [speed]
  const prices = coveredSpeeds.map((speed) =>
    index.rules.get(JSON.stringify([pair.model, group, speed]))
  )
  const priced = prices.length > 0 && prices.every(Boolean)
  if (route && !route.enabled)
    reasons.push('Enable the existing route before adding a target.')
  if (
    matches.length > 1 ||
    (target && target.upstream_model !== pair.upstream)
  ) {
    reasons.push('This channel already has a different target in this route.')
  }
  if (target && (target.weight <= 0 || !coveredSpeeds.length))
    reasons.push('Correct the target weight or speed in the route editor.')
  const active = Boolean(
    channel.enabled &&
    index.groups.has(group) &&
    route?.enabled &&
    target?.enabled &&
    target.upstream_model === pair.upstream &&
    target.weight > 0 &&
    coveredSpeeds.length
  )
  const done = active && priced
  const context = index.contexts.get(key(channel.id, pair.upstream))
  if (!done) {
    if (!base) reasons.push('Publish the official base price first.')
    if (base) {
      const destinations = (route?.targets ?? []).flatMap((t) => {
        const upstream = index.upstreams.get(t.upstream_id)
        return upstream && coveredSpeeds.some((s) => t.speeds.includes(s))
          ? [{ protocol: upstream.protocol, payload: t.payload }]
          : []
      })
      const issue = cacheWriteCoverageError(base, [
        ...destinations,
        { protocol: channel.protocol, payload: target?.payload },
      ])
      if (issue) reasons.push(issue)
    }
    if (!context) {
      const requirements = inputRequirements(channel)
      if (requirements.input && !positiveLimit(limits.input))
        reasons.push('Enter the upstream maximum input tokens.')
      if (requirements.output && !positiveLimit(limits.output))
        reasons.push('Enter the upstream maximum output tokens.')
      if (!limits.revision.trim())
        reasons.push('Enter the upstream contract revision.')
    }
  }
  return {
    base,
    route,
    target,
    context,
    speed,
    coveredSpeeds,
    prices,
    done,
    active,
    reasons,
  }
}

export function buildIntakeRoutes(
  config: NativeConfiguration,
  channelId: number,
  models: IntakeModel[],
  groups: string[],
  limits: Record<string, IntakeLimits>
) {
  const channel = config.routing.upstreams.find((row) => row.id === channelId)
  if (!channel || !models.length || !groups.length)
    throw new Error('Select models and routing groups.')
  if (
    new Set(models.map((row) => row.model)).size !== models.length ||
    new Set(models.map((row) => row.upstream)).size !== models.length
  )
    throw new Error(
      'Each selected upstream model needs a unique public model ID.'
    )
  const index = intakeIndex(config)
  const routes = [...config.routing.routes]
  const metadata = { ...config.bindings.models }
  const contexts: NativeContext[] = [...config.bindings.contexts]
  const touched = new Map<
    number,
    { model: string; upstream: string; group: string; channels: number[] }
  >()
  let routeId = routes.reduce((n, r) => Math.max(n, r.id), 0)
  let targetId = routes.reduce(
    (n, r) => r.targets.reduce((v, t) => Math.max(v, t.id), n),
    0
  )
  for (const pair of models) {
    const input = Object.hasOwn(limits, pair.upstream)
      ? limits[pair.upstream]
      : emptyLimits
    for (const group of new Set(groups)) {
      const state = groupIntake(config, index, channel, pair, group, input)
      if (state.reasons.length) throw new Error(state.reasons[0])
      if (state.done) continue
      const previous = state.route
      const id = previous?.id ?? ++routeId
      const target = state.target
        ? { ...state.target, enabled: true }
        : {
            id: ++targetId,
            upstream_id: channel.id,
            upstream_model: pair.upstream,
            enabled: true,
            priority: 0,
            weight: 1,
            speeds: [state.speed],
            timeout_ms: newNativeTargetTimeoutMs(config),
            payload: 'adapt',
          }
      if (!Number.isSafeInteger(id) || !Number.isSafeInteger(target.id))
        throw new Error('No safe route or target ID is available.')
      const route: NativeModelRoute = previous
        ? {
            ...previous,
            targets: state.target
              ? previous.targets.map((t) => (t.id === target.id ? target : t))
              : [...previous.targets, target],
          }
        : {
            id,
            model: pair.model,
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
      if (previous) routes[routes.indexOf(previous)] = route
      else {
        routes.push(route)
        metadata[id] = {
          display_name: defaultModelDisplayName(pair.model),
          description: '',
          vendor_id: null,
        }
      }
      if (
        !state.context &&
        !contexts.some(
          (c) =>
            c.upstream_id === channelId && c.upstream_model === pair.upstream
        )
      ) {
        const needs = inputRequirements(channel)
        contexts.push({
          ...newNativeContext(channel, pair.upstream),
          max_input_tokens: needs.input ? Number(input.input) : 0,
          max_output_tokens: needs.output ? Number(input.output) : 0,
          contract_revision: input.revision.trim(),
        })
      }
      touched.set(id, {
        ...pair,
        group,
        channels: [...new Set(route.targets.map((t) => t.upstream_id))],
      })
    }
  }
  if (!touched.size)
    throw new Error(
      'The selected routes and group prices are already published.'
    )
  if (
    ![config.version.routing + 1, config.version.pricing + 1].every(
      Number.isSafeInteger
    )
  )
    throw new Error('The routing snapshot changed. Reload before continuing.')
  const body = {
    expected_version: config.version,
    routing: { ...config.routing, version: config.version.routing + 1, routes },
    prices: { ...config.prices, version: config.version.pricing + 1 },
    bindings: { ...config.bindings, contexts, models: metadata },
  }
  return { body, touched }
}

export async function reviewIntakeRoutes(
  scope: NativeRoutingScope,
  displayed: NativeConfiguration,
  channelId: number,
  pairs: IntakeModel[],
  groups: string[],
  limits: Record<string, IntakeLimits>
) {
  const path = `${NATIVE_ROOT}/configuration`
  const raw = await nativeRequest<NativeConfiguration>(scope, 'GET', path)
  if (!samePriceRule(raw.version, displayed.version))
    throw new Error('The routing snapshot changed. Reload before continuing.')
  const plan = buildIntakeRoutes(raw, channelId, pairs, groups, limits)
  const result = await nativeRequest<
    Pick<NativeConfiguration, 'routing' | 'prices' | 'bindings'> & {
      saved: boolean
      confirmation_digest: string
      expected_version: NativeConfiguration['version']
    }
  >(scope, 'POST', path + '/preview', plan.body)
  if (
    result.saved !== false ||
    !samePriceRule(result.expected_version, raw.version) ||
    result.routing?.version !== plan.body.routing.version ||
    result.prices?.version !== plan.body.prices.version ||
    !Array.isArray(result.routing.routes) ||
    !Array.isArray(result.prices.rules) ||
    !/^[a-f0-9]{64}$/.test(result.confirmation_digest)
  )
    throw new Error('Invalid route publication preview')
  const selected = new Set(
    [...plan.touched.values()].map((p) => key(p.model, p.group))
  )
  const unchanged = (rules: NativeConfiguration['prices']['rules']) =>
    rules.filter((r) => !selected.has(key(r.model, r.group)))
  if (
    !samePriceRule(
      unchanged(raw.prices.rules),
      unchanged(result.prices.rules)
    ) ||
    !samePriceRule(
      raw.routing.routes.filter((r) => !plan.touched.has(r.id)),
      result.routing.routes.filter((r) => !plan.touched.has(r.id))
    )
  ) {
    throw new Error('The preview changes unselected routes or prices.')
  }
  const summaries = [...plan.touched].map(([id, item]) => {
    const route = result.routing.routes.find(
      (r) => r.id === id && r.model === item.model && r.group === item.group
    )
    const target = route?.targets.find(
      (r) =>
        r.upstream_id === channelId &&
        r.upstream_model === item.upstream &&
        r.enabled &&
        r.weight > 0
    )
    const speeds =
      target?.speeds.filter((speed) => route?.speeds.includes(speed)) ?? []
    if (
      !route?.enabled ||
      !speeds.length ||
      !speeds.every((speed) =>
        result.prices.rules.some(
          (r) =>
            r.model === item.model &&
            r.group === item.group &&
            r.speed === speed
        )
      )
    ) {
      throw new Error(
        'The preview has no active group price for a selected route.'
      )
    }
    const rules = result.prices.rules
      .filter(
        (r) =>
          r.model === item.model &&
          r.group === item.group &&
          route.speeds.includes(String(r.speed))
      )
      .map((rawRule) => {
        const parsed = nativePriceRuleSchema.safeParse(rawRule)
        if (!parsed.success)
          throw new Error('Invalid route publication preview')
        return parsed.data
      })
    return { ...item, rules }
  })
  return { ...plan, summaries, digest: result.confirmation_digest }
}

export async function publishIntakeRoutes(
  scope: NativeRoutingScope,
  review: Awaited<ReturnType<typeof reviewIntakeRoutes>>
) {
  return nativeRequest(scope, 'PUT', `${NATIVE_ROOT}/configuration`, {
    ...review.body,
    confirmation_digest: review.digest,
  })
}
