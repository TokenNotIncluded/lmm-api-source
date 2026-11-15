/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import type { InternalAxiosRequestConfig } from 'axios'

import type { NativeConfiguration } from '@/features/channels/native-api'
import {
  DEFAULT_NATIVE_USERS_QUERY,
  nativeUsersQuery,
} from '@/features/native-users/contract'
import {
  nativeListFixture,
  nativeStatusFixture,
  nativeUserFixture,
} from '@/features/native-users/test-fixtures'
import { ROLE } from '@/lib/roles'
import { useAuthStore } from '@/stores/auth-store'

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
      data: ['store', 'support', 'toolmarket', 'remote-control'].map(
        (name, index) => ({
          name,
          version: '1.0.0',
          api_version: 1,
          available: index !== 3,
          installed: index !== 3,
          enabled: index < 2,
          registration_sha256: String(index + 1).repeat(64),
        })
      ),
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
  if (url.pathname === '/core/v1/admin/users') {
    const query = nativeUsersQuery({
      ...DEFAULT_NATIVE_USERS_QUERY,
      ...config.params,
    })
    const users = [
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
    ].filter(
      (user) =>
        (!query.keyword ||
          [user.id, user.username, user.display_name, user.email].some(
            (value) => value?.includes(query.keyword)
          )) &&
        (!query.status || (query.status === 'active') === user.active) &&
        (!query.role || query.role === user.platform_role) &&
        (!query.group || user.groups.includes(query.group)) &&
        (!query.service_level ||
          String(user.service_level) === query.service_level)
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
