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
/**
 * Utility functions for usage logs filters
 */
import { LOG_CATEGORY_LABELS } from '../constants'
import type {
  LogCategory,
  LogFilters,
  CommonLogFilters,
  DrawingLogFilters,
  TaskLogFilters,
} from '../types'

// ============================================================================
// Filter Building Functions
// ============================================================================

/**
 * Build search params from filters based on log category
 */
export function buildSearchParams(
  filters: LogFilters,
  logCategory: LogCategory
): Record<string, unknown> {
  const baseParams: Record<string, unknown> = {
    ...(filters.startTime && { startTime: filters.startTime.getTime() }),
    ...(filters.endTime && { endTime: filters.endTime.getTime() }),
    ...(filters.channel && { channel: filters.channel }),
  }

  switch (logCategory) {
    case 'common': {
      const commonFilters = filters as CommonLogFilters
      return {
        ...baseParams,
        ...(commonFilters.model && { model: commonFilters.model }),
        ...(commonFilters.token && { token: commonFilters.token }),
        ...(commonFilters.group && { group: commonFilters.group }),
        ...(commonFilters.username && { username: commonFilters.username }),
        ...(commonFilters.requestId && { requestId: commonFilters.requestId }),
        ...(commonFilters.upstreamRequestId && {
          upstreamRequestId: commonFilters.upstreamRequestId,
        }),
        ...(commonFilters.status && { status: commonFilters.status }),
        ...(commonFilters.minDurationMs?.trim() && {
          minDurationMs: commonFilters.minDurationMs.trim(),
        }),
        ...(commonFilters.maxDurationMs?.trim() && {
          maxDurationMs: commonFilters.maxDurationMs.trim(),
        }),
        ...(commonFilters.sortOrder && { sortOrder: commonFilters.sortOrder }),
      }
    }
    case 'drawing': {
      const drawingFilters = filters as DrawingLogFilters
      return {
        ...baseParams,
        ...(drawingFilters.mjId?.trim() && {
          filter: drawingFilters.mjId.trim(),
        }),
        ...(drawingFilters.status && { status: drawingFilters.status }),
        ...(drawingFilters.action?.trim() && {
          action: drawingFilters.action.trim(),
        }),
      }
    }
    case 'task': {
      const taskFilters = filters as TaskLogFilters
      return {
        ...baseParams,
        ...(taskFilters.taskId?.trim() && {
          filter: taskFilters.taskId.trim(),
        }),
        ...(taskFilters.status && { status: taskFilters.status }),
        ...(taskFilters.action?.trim() && {
          action: taskFilters.action.trim(),
        }),
        ...(taskFilters.platform?.trim() && {
          platform: taskFilters.platform.trim(),
        }),
      }
    }
    default:
      return baseParams
  }
}

/**
 * Get log category display name
 */
export function getLogCategoryLabel(category: LogCategory): string {
  return LOG_CATEGORY_LABELS[category]
}

export function validateCommonLogFilters(
  filters: CommonLogFilters,
  isAdmin: boolean,
  historical: boolean
) {
  if (
    (filters.startTime && !Number.isFinite(filters.startTime.getTime())) ||
    (filters.endTime && !Number.isFinite(filters.endTime.getTime())) ||
    (filters.startTime &&
      filters.endTime &&
      filters.startTime > filters.endTime)
  ) {
    return 'Choose a valid time range with the start before the end.'
  }
  if (
    isAdmin &&
    filters.channel &&
    (!/^\d+$/.test(filters.channel) ||
      !Number.isSafeInteger(Number(filters.channel)) ||
      Number(filters.channel) < 1)
  ) {
    return 'Channel ID must be a positive integer.'
  }
  if (!historical) {
    const values = [
      filters.minDurationMs?.trim(),
      filters.maxDurationMs?.trim(),
    ]
    if (
      values.some(
        (value) =>
          value &&
          (!/^\d+(?:\.\d+)?$/.test(value) || Number(value) > 86_400_000)
      ) ||
      (values[0] && values[1] && Number(values[0]) > Number(values[1]))
    ) {
      return 'Use milliseconds from 0 to 86400000, with minimum no greater than maximum.'
    }
  }
  return undefined
}
