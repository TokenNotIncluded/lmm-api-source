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
/*
Copyright (C) 2026 LIghtJUNction
*/
import { useQuery } from '@tanstack/react-query'
import { useCallback, useMemo } from 'react'

import { useAuthStore } from '@/stores/auth-store'

import {
  getServiceHealth,
  getStatusDetectionMetrics,
  getStatusDetectionSummary,
} from '../api'
import { aggregateStatusGroups, latestStatusTimestamp } from '../lib/aggregate'

const MAX_MODELS = 48

type StatusDetectionOptions = {
  hours?: number
  model?: string
  group?: string
  vendor?: string
  modelVendors?: ReadonlyMap<string, string>
}

export function useStatusDetection(options: StatusDetectionOptions = {}) {
  const sessionScope = useAuthStore((state) => {
    const { user, session } = state.auth
    return user && session ? `${user.id}:${session.sid}` : null
  })
  const hours = options.hours ?? 24
  const modelFilter = options.model?.trim() ?? ''
  const groupFilter = options.group?.trim() ?? ''
  const vendorFilter = options.vendor?.trim() ?? ''
  const healthQuery = useQuery({
    queryKey: ['service-health'],
    queryFn: getServiceHealth,
    staleTime: 30_000,
    refetchInterval: 60_000,
    refetchIntervalInBackground: false,
    retry: false,
  })
  const summaryQuery = useQuery({
    queryKey: ['perf-metrics-summary', sessionScope, hours],
    queryFn: () => getStatusDetectionSummary(hours),
    enabled: sessionScope !== null,
    staleTime: 60 * 1000,
    retry: false,
  })

  const allModelNames = useMemo(
    () =>
      (summaryQuery.data?.data.models ?? [])
        .map((model) => model.model_name.trim())
        .filter(Boolean),
    [summaryQuery.data]
  )
  const modelNames = useMemo(
    () =>
      allModelNames
        .filter((modelName) => {
          if (modelFilter && modelName !== modelFilter) return false
          if (
            vendorFilter &&
            options.modelVendors?.get(modelName) !== vendorFilter
          ) {
            return false
          }
          return true
        })
        .slice(0, MAX_MODELS),
    [allModelNames, modelFilter, options.modelVendors, vendorFilter]
  )

  const detailsQuery = useQuery({
    queryKey: ['status-detection-metrics', sessionScope, hours, modelNames],
    queryFn: () => getStatusDetectionMetrics(modelNames, hours),
    enabled: sessionScope !== null && modelNames.length > 0,
    staleTime: 60 * 1000,
    retry: false,
  })

  const allGroups = useMemo(
    () => aggregateStatusGroups(detailsQuery.data?.entries ?? []),
    [detailsQuery.data?.entries]
  )
  const groups = useMemo(
    () =>
      groupFilter
        ? allGroups.filter((group) => group.group === groupFilter)
        : allGroups,
    [allGroups, groupFilter]
  )
  const latestTimestamp = useMemo(
    () => latestStatusTimestamp(detailsQuery.data?.entries ?? []),
    [detailsQuery.data?.entries]
  )

  const refresh = useCallback(async () => {
    await Promise.all([
      healthQuery.refetch(),
      ...(sessionScope !== null ? [summaryQuery.refetch()] : []),
      ...(sessionScope !== null && modelNames.length > 0
        ? [detailsQuery.refetch()]
        : []),
    ])
  }, [detailsQuery, healthQuery, modelNames.length, sessionScope, summaryQuery])

  return {
    health: healthQuery.data,
    healthError: healthQuery.error,
    healthLoading: healthQuery.isLoading,
    healthCheckedAt: healthQuery.dataUpdatedAt,
    requiresSignIn: sessionScope === null,
    coverage: summaryQuery.data,
    groups,
    availableGroups: allGroups.map((group) => group.group),
    availableModels: allModelNames,
    modelCount: modelNames.length,
    modelsWithData: detailsQuery.data?.entries.length ?? 0,
    failedModelCount: detailsQuery.data?.failedModels.length ?? 0,
    isLoading: summaryQuery.isLoading || detailsQuery.isLoading,
    isFetching:
      healthQuery.isFetching ||
      summaryQuery.isFetching ||
      detailsQuery.isFetching,
    error: summaryQuery.error ?? detailsQuery.error,
    hasSummary: Boolean(summaryQuery.data?.success),
    latestTimestamp,
    hours,
    refresh,
  }
}
