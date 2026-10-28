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
*/
import { api } from '@/lib/api'

import type { SecurityPolicyResponse, SecurityStatsResponse } from './types'

export const SECURITY_POLICY_ENDPOINT = '/api/security/policy'
export const SECURITY_STATS_ENDPOINT = '/api/security/stats'

export async function getSecurityPolicy(): Promise<SecurityPolicyResponse> {
  const res = await api.get<SecurityPolicyResponse>(SECURITY_POLICY_ENDPOINT, {
    skipBusinessError: true,
    skipErrorHandler: true,
  })
  return res.data
}

export async function getSecurityStats(): Promise<SecurityStatsResponse> {
  const res = await api.get<SecurityStatsResponse>(SECURITY_STATS_ENDPOINT, {
    skipBusinessError: true,
    skipErrorHandler: true,
  })
  return res.data
}
