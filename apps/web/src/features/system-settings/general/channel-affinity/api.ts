/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.

For commercial licensing, please contact support@quantumnous.com
*/
import {
  NATIVE_ROOT,
  readNativeRoutingScope,
} from '@/features/channels/native-api'
import { api } from '@/lib/api'

import type { CacheStats } from './types'

function cacheRequestOptions() {
  const scope = readNativeRoutingScope()
  if (!scope) throw new Error('Administrator access required')
  return {
    authScope: { userId: scope.userId, sessionId: scope.sessionId },
    skipAuthRefresh: true,
    disableDuplicate: true,
  }
}

export async function getCacheStats(): Promise<{
  success: boolean
  message?: string
  data?: CacheStats
}> {
  const res = await api.get(
    `${NATIVE_ROOT}/channel-affinity/cache`,
    cacheRequestOptions()
  )
  return res.data
}

export async function clearAllCache(): Promise<{
  success: boolean
  message?: string
}> {
  const res = await api.delete(`${NATIVE_ROOT}/channel-affinity/cache`, {
    ...cacheRequestOptions(),
    params: { all: true },
  })
  return res.data
}

export async function clearRuleCache(
  ruleName: string
): Promise<{ success: boolean; message?: string }> {
  const res = await api.delete(`${NATIVE_ROOT}/channel-affinity/cache`, {
    ...cacheRequestOptions(),
    params: { rule_name: ruleName },
  })
  return res.data
}

export async function getAffinityUsageCache(params: {
  rule_name: string
  using_group: string
  key_hint: string
  key_fp: string
}): Promise<{ success: boolean; message?: string; data?: unknown }> {
  const res = await api.get('/api/log/channel_affinity_usage_cache', {
    params,
    disableDuplicate: true,
  } as Record<string, unknown>)
  return res.data
}
