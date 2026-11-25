/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import type { NativeConfiguration, NativeModelRoute } from '../native-api'

export type ChannelPlanSpeed = 'standard' | 'fast' | 'ultrafast'
export type ChannelPlanModel = {
  upstreamModel: string
  publicModel: string
  groups?: readonly string[]
  placement?: 'backup' | 'primary'
}
export type ChannelPlanPrice = {
  model: string
  group: string
  speed: ChannelPlanSpeed
}
export type NativeChannelPlanInput = {
  channelId: number
  models: readonly ChannelPlanModel[]
  groups: readonly string[]
  speeds: readonly ChannelPlanSpeed[]
  placement?: 'backup' | 'primary'
  priceTemplateRuleId?: number
}
export type ChannelPlanConflict =
  | 'invalid_model'
  | 'unknown_group'
  | 'duplicate_public_model'
  | 'alias_in_use'
  | 'different_upstream_model'
  | 'unsupported_speed'
  | 'backup_priority_exhausted'
  | 'route_target_limit'
  | 'no_primary_priority'
  | 'retry_budget_exhausted'
export type ChannelPlanRow = ChannelPlanModel & {
  group: string
  routeId: number | null
  priority: number | null
  existingTargets: readonly NativeModelRoute['targets'][number][]
  failover: NativeModelRoute['failover'] | null
  failoverAfter: NativeModelRoute['failover'] | null
  addedSpeeds: ChannelPlanSpeed[]
  action:
    | 'create_route'
    | 'add_target'
    | 'extend_target'
    | 'existing'
    | 'conflict'
  conflict?: ChannelPlanConflict
  missingPrices: ChannelPlanPrice[]
}
export type NativeChannelPlan = {
  rows: ChannelPlanRow[]
  missingPrices: ChannelPlanPrice[]
  proposedPrices: Array<ChannelPlanPrice & { templateRuleId: number }>
}

export function mergeChannelModelIds(
  selected: readonly string[],
  pasted: string
): string[] {
  return [
    ...new Set(
      [...selected, ...pasted.split(/[,，\r\n]+/)]
        .map((id) => id.trim())
        .filter(Boolean)
    ),
  ]
}

function validModel(value: string): boolean {
  return (
    /^[a-zA-Z0-9._\-/:@~]{1,200}$/.test(value) &&
    !value.endsWith('/fast') &&
    !value.endsWith('/ultrafast')
  )
}

const routeKey = (group: string, model: string) =>
  JSON.stringify([group, model])
const priceKey = (group: string, model: string, speed: string) =>
  JSON.stringify([group, model, speed])

/** Preview additions against one published snapshot. Never synthesize an enabled target or a price. */
export function planNativeChannelModels(
  configuration: NativeConfiguration,
  input: NativeChannelPlanInput
): NativeChannelPlan {
  const { routing, prices } = configuration
  const routes = new Map(
    routing.routes.map((route) => [routeKey(route.group, route.model), route])
  )
  const groups = new Set(routing.groups.map((group) => group.name))
  const enabledUpstreams = new Set(
    routing.upstreams
      .filter((upstream) => upstream.enabled)
      .map((upstream) => upstream.id)
  )
  const occupiedAliases = new Set<string>()
  for (const record of routing.aliases) {
    if (
      record &&
      typeof record === 'object' &&
      'group' in record &&
      typeof record.group === 'string' &&
      'alias' in record &&
      typeof record.alias === 'string'
    ) {
      occupiedAliases.add(routeKey(record.group, record.alias))
    }
  }
  const publishedPrices = new Set<string>()
  for (const rule of prices.rules) {
    if (
      typeof rule.model === 'string' &&
      typeof rule.group === 'string' &&
      typeof rule.speed === 'string'
    ) {
      publishedPrices.add(priceKey(rule.group, rule.model, rule.speed))
    }
  }
  // A public ID belongs to a group. A per-model group selection replaces the bulk choice.
  const selections = new Map<
    string,
    { group: string; models: Map<string, ChannelPlanModel> }
  >()
  for (const model of input.models) {
    for (const group of new Set(model.groups ?? input.groups)) {
      const key = routeKey(group, model.publicModel)
      const selection = selections.get(key) ?? {
        group,
        models: new Map<string, ChannelPlanModel>(),
      }
      selection.models.set(model.upstreamModel, model)
      selections.set(key, selection)
    }
  }
  const rows: ChannelPlanRow[] = []
  const missingPrices: ChannelPlanPrice[] = []
  const speeds = [...new Set(input.speeds)]
  for (const [key, selection] of selections) {
    const { group } = selection
    const route = routes.get(key)
    const target = route?.targets.find(
      (item) => item.upstream_id === input.channelId
    )
    for (const {
      publicModel,
      upstreamModel,
      placement = input.placement ?? 'backup',
    } of selection.models.values()) {
      const previous = route?.failover ?? null
      const incumbents = route?.targets ?? []
      let priority = target?.priority ?? 0
      let requiredAttempts = 1
      const addedSpeeds = speeds.filter(
        (speed) => !target?.speeds.includes(speed)
      )
      if (!target && incumbents.length > 0) {
        priority =
          placement === 'primary'
            ? Math.min(...incumbents.map((binding) => binding.priority)) - 1
            : Math.max(...incumbents.map((binding) => binding.priority)) + 1
        for (const speed of speeds) {
          const eligible = incumbents.filter(
            (binding) =>
              binding.enabled &&
              binding.weight > 0 &&
              enabledUpstreams.has(binding.upstream_id) &&
              binding.speeds.includes(speed)
          ).length
          const attempts =
            placement === 'primary' ? (eligible > 0 ? 2 : 1) : eligible + 1
          requiredAttempts = Math.max(requiredAttempts, attempts)
        }
      }
      let conflict: ChannelPlanConflict | undefined
      if (!validModel(upstreamModel) || !validModel(publicModel)) {
        conflict = 'invalid_model'
      } else if (!groups.has(group)) conflict = 'unknown_group'
      else if (selection.models.size > 1) conflict = 'duplicate_public_model'
      else if (occupiedAliases.has(key)) conflict = 'alias_in_use'
      else if (target && target.upstream_model !== upstreamModel) {
        conflict = 'different_upstream_model'
      } else if (priority > 65535) conflict = 'backup_priority_exhausted'
      else if (priority < 0) conflict = 'no_primary_priority'
      else if (requiredAttempts > 8) conflict = 'retry_budget_exhausted'
      else if (!target && incumbents.length >= 64) {
        conflict = 'route_target_limit'
      } else if (
        speeds.length === 0 ||
        (route && speeds.some((speed) => !route.speeds.includes(speed)))
      ) {
        conflict = 'unsupported_speed'
      }
      const failoverAfter = conflict
        ? null
        : {
            max_attempts: Math.max(
              previous?.max_attempts ?? 1,
              requiredAttempts
            ),
            retry_on: previous?.retry_on.length
              ? [...previous.retry_on]
              : requiredAttempts > 1
                ? ['connect']
                : [],
            allow_replay_after_send: previous?.allow_replay_after_send ?? false,
          }

      const missing = conflict
        ? []
        : speeds
            .filter(
              (speed) =>
                !publishedPrices.has(priceKey(group, publicModel, speed))
            )
            .map((speed) => ({ model: publicModel, group, speed }))
      missingPrices.push(...missing)
      rows.push({
        upstreamModel,
        publicModel,
        group,
        routeId: route?.id ?? null,
        priority: conflict ? null : priority,
        existingTargets: route?.targets ?? [],
        failover: previous,
        failoverAfter,
        addedSpeeds: conflict ? [] : addedSpeeds,
        action: conflict
          ? 'conflict'
          : target
            ? addedSpeeds.length
              ? 'extend_target'
              : 'existing'
            : route
              ? 'add_target'
              : 'create_route',
        ...(conflict ? { conflict } : {}),
        missingPrices: missing,
      })
    }
  }
  const templateId = input.priceTemplateRuleId
  const hasTemplate =
    templateId !== undefined &&
    prices.rules.some((rule) => rule.id === templateId)
  return {
    rows,
    missingPrices,
    proposedPrices: hasTemplate
      ? missingPrices.map((price) => ({ ...price, templateRuleId: templateId }))
      : [],
  }
}

/** A discovery snapshot can be incomplete. Missing IDs are informational and never removals. */
export function diffNativeChannelCatalog(
  configuration: NativeConfiguration,
  channelId: number,
  discovered: readonly string[]
): { added: string[]; retained: string[]; disappeared: string[] } {
  const bound = new Set<string>()
  for (const route of configuration.routing.routes) {
    for (const target of route.targets) {
      if (target.upstream_id === channelId) bound.add(target.upstream_model)
    }
  }
  const upstream = new Set(mergeChannelModelIds(discovered, ''))
  return {
    added: [...upstream].filter((id) => !bound.has(id)),
    retained: [...upstream].filter((id) => bound.has(id)),
    disappeared: [...bound].filter((id) => !upstream.has(id)),
  }
}
