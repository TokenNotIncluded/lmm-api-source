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
import { api } from '@/lib/api'

import type {
  RankingPeriod,
  RankingsSnapshot,
  UserUsageRankingsSnapshot,
} from './types'

type RankingsResponse<T> = {
  success: boolean
  message?: string
  code?: string
  error?: { code?: string }
  data: T
}

export class RankingsResponseError extends Error {
  readonly code?: string

  constructor(code?: string) {
    super('Unable to load rankings data')
    this.code = code
  }
}

async function readRankings<T>(
  path: string,
  period: RankingPeriod
): Promise<RankingsResponse<T>> {
  const { data } = await api.get<RankingsResponse<T>>(path, {
    params: { period },
    skipErrorHandler: true,
    skipBusinessError: true,
  })
  if (data?.success !== true || data.data == null) {
    throw new RankingsResponseError(data?.code ?? data?.error?.code)
  }
  return data
}

export function getRankings(period: RankingPeriod) {
  return readRankings<RankingsSnapshot>('/api/rankings', period)
}

export function getUserUsageRankings(period: RankingPeriod) {
  return readRankings<UserUsageRankingsSnapshot>('/api/rankings/users', period)
}
