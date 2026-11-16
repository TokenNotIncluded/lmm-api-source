/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { AxiosError, type AxiosAdapter } from 'axios'

import type {
  NativeConfiguration,
  NativeKeyList,
} from '@/features/channels/native-api'
import { ROLE } from '@/lib/roles'
import { useAuthStore } from '@/stores/auth-store'

// Separate from the read-only console gallery. This adapter is installed only
// by debug-main on loopback with management_review=1. Nothing reaches a backend.
// Every identity, endpoint, label and credential input in this review is synthetic.
type ReviewCall = { method: string; path: string; payload: unknown }
type ReviewTrace = {
  synthetic: true
  calls: ReviewCall[]
  loseNextMutationResponse: boolean
  snapshot: () => unknown
}
type ReviewWindow = Window & { __LMM_MANAGEMENT_REVIEW__?: ReviewTrace }
type ReviewPlugin = {
  name: string
  version: string
  api_version: number
  available: boolean
  installed: boolean
  enabled: boolean
  registration_sha256: string
  requires: string[]
  navigation_paths: string[]
}

export function withManagementReviewFixtures(next: AxiosAdapter): AxiosAdapter {
  if (
    window.location.protocol !== 'http:' ||
    window.location.hostname !== '127.0.0.1' ||
    new URLSearchParams(window.location.search).get('management_review') !== '1'
  ) {
    return next
  }
  const plugins: ReviewPlugin[] = [
    'store',
    'support',
    'toolmarket',
    'remote-control',
  ].map((name, index) => ({
    name,
    version: '1.0.0',
    api_version: 1,
    available: index !== 3,
    installed: index < 2,
    enabled: index < 2,
    registration_sha256: String(index + 1).repeat(64),
    requires: name === 'toolmarket' ? ['store'] : [],
    navigation_paths:
      name === 'store' ? ['/store'] : name === 'support' ? ['/support'] : [],
  }))
  const configuration: NativeConfiguration = {
    version: { routing: 1, pricing: 1 },
    channel_id_high_water: 2,
    secret_storage_enabled: true,
    routing: {
      version: 1,
      upstreams: [1, 2].map((id) => ({
        id,
        endpoint: `https://provider-${id}.example.invalid/v1/`,
        protocol: 'openai_chat',
        enabled: id === 1,
        credential: { id, revision: 1 },
      })),
      routes: [
        {
          id: 1,
          model: 'synthetic-chat',
          group: 'default',
          enabled: true,
          speeds: ['standard'],
          targets: [
            {
              id: 1,
              upstream_id: 1,
              upstream_model: 'synthetic-upstream-chat',
              enabled: true,
              priority: 0,
              weight: 100,
              speeds: ['standard'],
              payload: 'adapt',
              timeout_ms: 30000,
            },
          ],
          failover: {
            max_attempts: 1,
            retry_on: [],
            allow_replay_after_send: false,
          },
        },
      ],
      groups: [{ name: 'default', enabled: true }],
      aliases: [],
    },
    prices: { version: 1, rules: [] },
    bindings: {
      names: {
        1: 'Synthetic primary provider',
        2: 'Synthetic fallback provider',
      },
      tags: { 1: 'synthetic-long-routing-label-'.repeat(4) },
      contexts: [
        {
          upstream_id: 1,
          upstream_model: 'synthetic-upstream-chat',
          protocol: 'openai_chat',
          endpoint: 'https://provider-1.example.invalid/v1/',
          max_input_tokens: 128000,
          max_output_tokens: 4096,
          contract_revision: 'synthetic-review-v1',
        },
      ],
      vendors: {},
      models: {
        1: {
          display_name: 'Synthetic chat model',
          description: 'Local fixture; no provider is called.',
          vendor_id: null,
        },
      },
      discoveries: {
        1: {
          models: ['synthetic-upstream-chat', 'synthetic-upstream-vision'],
          observed_at: 1790035200,
        },
      },
      price_drafts: {},
    },
  }
  const keyLists = new Map<number, NativeKeyList>(
    [1, 2].map((id) => [
      id,
      {
        version: { ...configuration.version },
        active_reference: { id, revision: 1 },
        polling: false,
        keys: [
          {
            reference: { id, revision: 1 },
            label: `Synthetic original key ${id}`,
            created_at: '2026-11-16T00:00:00Z',
            enabled: true,
          },
        ],
        plaintext_available: false,
      },
    ])
  )
  let nextKeyId = 10
  const trace: ReviewTrace = {
    synthetic: true,
    calls: [],
    loseNextMutationResponse: false,
    snapshot: () =>
      structuredClone({
        plugins,
        version: configuration.version,
        keys: [...keyLists.entries()],
      }),
  }
  ;(window as ReviewWindow).__LMM_MANAGEMENT_REVIEW__ = trace

  return async (config) => {
    const url = new URL(config.url ?? '', window.location.origin)
    const method = (config.method ?? 'get').toUpperCase()
    const role = useAuthStore.getState().auth.user?.role
    if (
      url.origin !== window.location.origin ||
      (role !== ROLE.ADMIN && role !== ROLE.SUPER_ADMIN)
    ) {
      return next(config)
    }
    const pluginPath =
      url.pathname === '/api/extensions/plugins' ||
      url.pathname.startsWith('/api/extensions/plugins/')
    const keyPath = /^\/core\/v1\/admin\/routing\/channels\/(\d+)\/keys$/.exec(
      url.pathname
    )
    const modelPreview =
      method === 'POST' &&
      url.pathname === '/core/v1/admin/routing/models/preview'
    const knownRead =
      method === 'GET' &&
      ['/api/channel/ops', '/core/v1/admin/routing/configuration'].includes(
        url.pathname
      )
    if (!pluginPath && !keyPath && !knownRead && !modelPreview) {
      return next(config)
    }

    let payload: Record<string, unknown> = {}
    if (config.data !== undefined && config.data !== '') {
      payload =
        typeof config.data === 'string' ? JSON.parse(config.data) : config.data
    }
    const recorded =
      keyPath && method !== 'GET'
        ? {
            operation: payload.operation,
            expected_version: payload.expected_version,
            polling: payload.polling,
            enabled: payload.enabled,
            reference: payload.reference,
            key_count: Array.isArray(payload.keys)
              ? payload.keys.length
              : payload.secret
                ? 1
                : 0,
            label: payload.label,
          }
        : payload
    trace.calls.push({
      method,
      path: url.pathname,
      payload: structuredClone(recorded),
    })
    if (trace.calls.length > 256) trace.calls.shift()
    const respond = (data: unknown) => ({
      data: { success: true, data: structuredClone(data) },
      status: 200,
      statusText: 'OK',
      headers: {},
      config,
    })
    const reject = (message: string, status = 409): never => {
      throw new AxiosError(message, 'ERR_BAD_REQUEST', config, undefined, {
        data: { success: false, message, code: 'SYNTHETIC_REVIEW_REJECTION' },
        status,
        statusText: 'Rejected',
        headers: {},
        config,
      })
    }
    const mutationResult = (data: unknown) => {
      if (trace.loseNextMutationResponse) {
        trace.loseNextMutationResponse = false
        throw new AxiosError(
          'Synthetic response loss after the operation committed.',
          'ECONNABORTED',
          config
        )
      }
      return respond(data)
    }
    if (method === 'GET' && url.pathname === '/api/channel/ops') {
      return respond({ native_configuration: true, native_secrets: true })
    }
    if (
      method === 'GET' &&
      url.pathname === '/core/v1/admin/routing/configuration'
    ) {
      return respond(configuration)
    }
    if (modelPreview) {
      return respond({
        confirmation_digest: 'a'.repeat(64),
        synthetic: true,
        message: 'Local draft preview only; no backend mutation.',
      })
    }

    if (pluginPath) {
      if (method === 'GET' && url.pathname === '/api/extensions/plugins') {
        return respond(plugins)
      }
      if (method === 'POST' && url.pathname.endsWith('/reload')) {
        return mutationResult(null)
      }
      if (method === 'POST' && url.pathname.endsWith('/install')) {
        const plugin = plugins.find((item) => item.name === payload.name)
        if (
          !plugin ||
          !plugin.available ||
          plugin.version !== payload.version ||
          plugin.api_version !== payload.api_version ||
          plugin.registration_sha256 !== payload.registration_sha256
        ) {
          return reject('Synthetic plugin manifest mismatch.', 422)
        }
        plugin.installed = true
        return mutationResult(plugin)
      }
      const match =
        /^\/api\/extensions\/plugins\/([^/]+)(?:\/(enable|disable))?$/.exec(
          url.pathname
        )
      const plugin =
        match &&
        plugins.find((item) => item.name === decodeURIComponent(match[1]))
      if (!match || !plugin) return reject('Synthetic plugin not found.', 404)
      if (method === 'POST' && match[2] === 'enable') {
        if (
          !plugin.available ||
          !plugin.installed ||
          plugin.requires.some(
            (name) => !plugins.find((item) => item.name === name)?.enabled
          )
        ) {
          return reject('Synthetic plugin dependency is not enabled.')
        }
        plugin.enabled = true
      } else if (method === 'POST' && match[2] === 'disable') {
        if (
          plugins.some(
            (item) => item.enabled && item.requires.includes(plugin.name)
          )
        ) {
          return reject('Synthetic enabled plugin depends on this plugin.')
        }
        plugin.enabled = false
      } else if (method === 'DELETE' && !match[2]) {
        if (plugin.enabled) {
          return reject('Disable the synthetic plugin before uninstalling it.')
        }
        plugin.installed = false
      } else return reject('Unexpected synthetic plugin operation.', 405)
      return mutationResult(plugin)
    }
    if (keyPath) {
      const channelId = Number(keyPath[1])
      const list = keyLists.get(channelId)
      if (!list) return reject('Synthetic channel not found.', 404)
      list.version = { ...configuration.version }
      if (method === 'GET') return respond(list)
      if (method !== 'POST') {
        return reject('Unexpected synthetic key operation.', 405)
      }
      const expected = payload.expected_version as
        | { routing?: number; pricing?: number }
        | undefined
      if (
        expected?.routing !== configuration.version.routing ||
        expected?.pricing !== configuration.version.pricing
      ) {
        return reject('Synthetic routing version changed.')
      }
      let reference = list.active_reference
      let importedCount: number | undefined
      const add = (label: string) => {
        reference = { id: nextKeyId++, revision: 1 }
        list.keys.push({
          reference,
          label,
          created_at: '2026-11-16T00:00:00Z',
          enabled: true,
        })
      }
      if (payload.operation === 'add_many') {
        const keys = payload.keys as Array<{ label: string; secret: string }>
        if (
          !Array.isArray(keys) ||
          keys.length < 1 ||
          keys.length > 100 ||
          keys.some((key) => !key.label || !key.secret) ||
          new Set(keys.map((key) => key.secret)).size !== keys.length
        ) {
          return reject('Enter 1 to 100 different synthetic keys.', 422)
        }
        for (const key of keys) add(key.label)
        list.polling = payload.polling === true
        importedCount = keys.length
      } else if (payload.operation === 'add') {
        if (
          typeof payload.label !== 'string' ||
          !payload.label ||
          typeof payload.secret !== 'string' ||
          !payload.secret
        ) {
          return reject('Synthetic key label and value are required.', 422)
        }
        add(payload.label)
        if (payload.activate) list.active_reference = reference
      } else if (payload.operation === 'set_polling') {
        list.polling = payload.enabled === true
      } else {
        const input = payload.reference as
          | { id?: number; revision?: number }
          | undefined
        const key = list.keys.find(
          (item) =>
            item.reference.id === input?.id &&
            item.reference.revision === input?.revision
        )
        if (!key) return reject('Synthetic key reference not found.', 404)
        reference = key.reference
        if (payload.operation === 'activate') list.active_reference = reference
        else if (payload.operation === 'set_enabled') {
          key.enabled = payload.enabled === true
        } else if (payload.operation === 'remove') {
          list.keys = list.keys.filter((item) => item !== key)
        } else return reject('Unexpected synthetic key operation.', 405)
      }
      configuration.version.routing += 1
      configuration.routing.version = configuration.version.routing
      list.version = { ...configuration.version }
      return mutationResult({
        version: configuration.version,
        reference,
        ...(importedCount === undefined
          ? {}
          : { imported_count: importedCount }),
      })
    }
    return next(config)
  }
}
