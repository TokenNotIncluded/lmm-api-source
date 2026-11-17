/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import type { InternalAxiosRequestConfig } from 'axios'

import type { NativeConfiguration } from '@/features/channels/native-api'
import {
  DEFAULT_NATIVE_USERS_QUERY,
  nativeUsersQuery,
} from '@/features/native-users/contract'
import { nativeUserStatisticsFixture } from '@/features/native-users/detail-test-fixtures'
import {
  nativeListFixture,
  nativeStatusFixture,
  nativeUserFixture,
  nativeUsageFixture,
} from '@/features/native-users/test-fixtures'
import { ROLE } from '@/lib/roles'
import { useAuthStore } from '@/stores/auth-store'

// This account is shared by the local usage rows and the exact native reader.
export const USAGE_REVIEW_USER = nativeUserFixture({
  id: '1006',
  personal_account_id: '2006',
  auth_version: '1',
  username: 'preview-user',
  display_name: 'Synthetic usage account',
  email: 'preview-user@example.invalid',
  groups: ['default'],
  core_account_created_at_unix_ms: 1790035200000,
  registered_at_unix_ms: 1790035200000,
  wallet: {
    status: 'available',
    unit: 'core_credit',
    available_units: '275000',
    wallet_units: '275000',
    reserved_units: '4000',
    debt_units: '0',
    money_basis: 'unknown',
    money_available: false,
    currency: null,
    provenance: {
      reader: 'financial_profile.read_wallet.v1',
      user_id: '1006',
      personal_account_id: '2006',
    },
  },
})

function reviewUsers() {
  return [
    nativeUserFixture({ display_name: 'Synthetic account · layout preview' }),
    nativeUserFixture({
      id: '9007199254740994',
      personal_account_id: '9007199254740996',
      username: 'synthetic-disabled-user',
      email: 'disabled@example.invalid',
      active: false,
      status: 2,
      wallet: {
        status: 'unavailable',
        code: 'WALLET_PROJECTION_UNAVAILABLE',
      },
    }),
    structuredClone(USAGE_REVIEW_USER),
  ]
}

// Only imported by the loopback persona entry's console_review=1 catalog.
// These are synthetic reads; mutations continue to the fail-closed adapter.
export function adminReviewFixture(
  config: InternalAxiosRequestConfig
): unknown {
  const url = new URL(config.url ?? '', window.location.origin)
  const role = useAuthStore.getState().auth.user?.role
  if (
    (config.method ?? 'get').toUpperCase() !== 'GET' ||
    url.origin !== window.location.origin ||
    url.username ||
    url.password ||
    (role !== ROLE.ADMIN && role !== ROLE.SUPER_ADMIN)
  ) {
    return undefined
  }
  if (url.pathname === '/api/status') {
    return {
      success: true,
      data: {
        ...nativeStatusFixture.data,
        system_name: 'LMM Persona Lab',
        logo: '/logo.png',
        assistant: { enabled: false },
        announcements_enabled: false,
      },
    }
  }
  if (url.pathname === '/api/extensions/plugins') {
    return {
      success: true,
      data: ['store', 'support', 'toolmarket'].map((name, index) => ({
        name,
        version: '1.0.0',
        api_version: 1,
        available: index !== 3,
        installed: index !== 3,
        enabled: index < 2,
        registration_sha256: String(index + 1).repeat(64),
      })),
    }
  }
  if (url.pathname === '/api/channel/ops') {
    return {
      success: true,
      data: { native_configuration: true, native_secrets: false },
    }
  }
  if (url.pathname === '/core/v1/admin/routing/configuration') {
    const configuration: NativeConfiguration = {
      version: { routing: 1, pricing: 1 },
      secret_storage_enabled: false,
      routing: {
        version: 1,
        upstreams: [1, 2].map((id) => ({
          id,
          endpoint: `https://provider-${id}.example.invalid/v1/`,
          protocol: 'openai_chat',
          enabled: id === 1,
          credential: { id, revision: 1 },
        })),
        routes: [],
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
        contexts: [],
        vendors: {},
        models: {},
        discoveries: {},
        price_drafts: {},
      },
    }
    return { success: true, data: configuration }
  }
  if (url.pathname === '/core/v1/admin/instances') {
    const observedAt = Math.floor(Date.now() / 1000)
    return {
      items: [
        {
          boot_id: 'a'.repeat(32),
          instance_name: 'synthetic-core-primary',
          version: 'synthetic-review',
          phase: 'ready',
          in_flight: 4,
          models_ready: true,
          rpc_ready: true,
          started_at: observedAt - 86400,
          heartbeat_at: observedAt,
          online: true,
        },
        {
          boot_id: 'b'.repeat(32),
          instance_name: 'synthetic-core-standby',
          version: 'synthetic-review',
          phase: 'draining',
          in_flight: 0,
          models_ready: true,
          rpc_ready: false,
          started_at: observedAt - 172800,
          heartbeat_at: observedAt - 180,
          online: false,
        },
      ],
      truncated: false,
      stale_after_seconds: 45,
      retention_days: 0,
    }
  }
  if (url.pathname === '/core/v1/admin/system-resources') {
    return {
      schema_version: 1,
      observed_at_unix_ms: Date.now(),
      sample_window_ms: 1000,
      system: {
        logical_cpus: 8,
        load_average: { one: 0.42, five: 0.36, fifteen: 0.28 },
        cpu_usage_percent: 12.5,
        memory_total_bytes: 17179869184,
        memory_available_bytes: 12884901888,
        memory_used_bytes: 4294967296,
        memory_usage_percent: 25,
      },
      processes: {
        'lmm-core': {
          status: 'running',
          version: 'synthetic-core',
          pid: 3024,
          cpu_percent: 8.4,
          memory_bytes: 268435456,
          memory_percent: 1.5625,
          threads: 16,
        },
        'lmm-extensions': {
          status: 'running',
          version: 'synthetic-extensions',
          pid: 4176,
          cpu_percent: 2.1,
          memory_bytes: 134217728,
          memory_percent: 0.78125,
          threads: 8,
        },
      },
    }
  }
  if (url.pathname === '/core/v1/admin/users/groups') {
    const query = nativeUsersQuery({
      ...DEFAULT_NATIVE_USERS_QUERY,
      ...config.params,
    })
    const groups = [
      ...new Set(reviewUsers().flatMap((user) => user.groups)),
    ].sort()
    const start = (query.page - 1) * query.page_size
    return {
      version: 1,
      items: groups.slice(start, start + query.page_size),
      total: groups.length,
      page: query.page,
      page_size: query.page_size,
      source: 'enabled_explicit_user_routing_grants',
    }
  }
  const detail = /^\/core\/v1\/admin\/users\/([1-9]\d*)$/.exec(url.pathname)
  if (detail) {
    const user = reviewUsers().find((item) => item.id === detail[1])
    return user
      ? {
          version: 1,
          user,
          usage: nativeUsageFixture(),
          statistics: nativeUserStatisticsFixture(user),
        }
      : undefined
  }
  if (url.pathname === '/core/v1/admin/users') {
    const query = nativeUsersQuery({
      ...DEFAULT_NATIVE_USERS_QUERY,
      ...config.params,
    })
    const users = reviewUsers().filter(
      (user) =>
        (!query.keyword ||
          [user.id, user.username, user.display_name, user.email].some(
            (value) => value?.includes(query.keyword)
          )) &&
        (!query.status || (query.status === 'active') === user.active) &&
        (!query.role || query.role === user.platform_role) &&
        (!query.group || user.groups.includes(query.group)) &&
        (!query.team_id || user.teams.some((team) => team.id === query.team_id))
    )
    const start = (query.page - 1) * query.page_size
    return {
      ...nativeListFixture(
        users.slice(start, start + query.page_size),
        query.page,
        users.length
      ),
      page_size: query.page_size,
    }
  }
  return undefined
}
