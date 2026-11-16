/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { useQuery } from '@tanstack/react-query'
import { useEffect } from 'react'

import { useUsageLogsContext } from '../components/usage-logs-provider'
import type { FetchLogsConfig } from '../types'
import type { LogsPage } from './history-response'
import { fetchLogsByCategory } from './utils'

/** Select imported history once when the initial native result has no matches. */
export function useInitialUsageSource(
  nativeLogs: LogsPage | undefined,
  nativeReady: boolean,
  config: FetchLogsConfig
) {
  const { initialUsageSourcePending, resolveInitialUsageSource } =
    useUsageLogsContext()
  const eligible =
    initialUsageSourcePending &&
    !config.historical &&
    config.logCategory === 'common' &&
    nativeReady &&
    nativeLogs?.kind === 'common'

  const history = useQuery({
    queryKey: [
      'usage-initial-history',
      config.scope?.userId,
      config.scope?.sessionId,
      config.scope?.role,
      config.isAdmin,
      config.searchParams,
      config.columnFilters,
    ],
    queryFn: async ({ signal }) => {
      const result = await fetchLogsByCategory({
        ...config,
        historical: true,
        page: 1,
        pageSize: 1,
        signal,
      })
      return result.data?.kind === 'historical' && result.data.total > 0
    },
    enabled: eligible && nativeLogs.total === 0 && !!config.scope,
    retry: false,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
  })

  useEffect(() => {
    if (!eligible) return
    if (nativeLogs.total > 0 || history.isError) {
      resolveInitialUsageSource(false)
    } else if (history.isSuccess && !history.isFetching) {
      resolveInitialUsageSource(history.data)
    }
  }, [
    eligible,
    nativeLogs?.total,
    history.isError,
    history.isSuccess,
    history.isFetching,
    history.data,
    resolveInitialUsageSource,
  ])
}
