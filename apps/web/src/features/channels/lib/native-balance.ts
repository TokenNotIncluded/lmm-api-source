/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { isAxiosError } from 'axios'

import { getNativeChannelBalance, type NativeRoutingScope } from '../native-api'
import type { BalanceObservation } from '../types'

export function nativeBalanceFailureKey(error: unknown, probe = false) {
  const status = isAxiosError(error) ? error.response?.status : undefined
  return status === 501 && probe
    ? 'channelBalance.disabled'
    : status === 403
      ? 'channelBalance.denied'
      : status === 409
        ? 'channelUsage.identity'
        : 'channelBalance.error'
}

export type NativeBalanceRefreshResult =
  | { channelId: number; status: 'ready'; observation: BalanceObservation }
  | { channelId: number; status: 'error'; message: string }

/** One visible page, one request at a time, with the exact single-channel API. */
export async function refreshNativeChannelBalances(
  scope: NativeRoutingScope,
  channelIds: readonly number[],
  signal: AbortSignal,
  onResult: (result: NativeBalanceRefreshResult) => void
) {
  const ids = [...new Set(channelIds)]
  if (
    ids.length > 25 ||
    ids.some((id) => !Number.isSafeInteger(id) || id <= 0)
  ) {
    throw new Error('Select at most one page of valid channels.')
  }
  for (const channelId of ids) {
    if (signal.aborted) return
    let result: NativeBalanceRefreshResult
    try {
      const observation = await getNativeChannelBalance(
        scope,
        channelId,
        signal
      )
      result = { channelId, status: 'ready', observation }
    } catch (error: unknown) {
      result = {
        channelId,
        status: 'error',
        message: nativeBalanceFailureKey(error, true),
      }
    }
    if (signal.aborted) return
    onResult(result)
    if (
      result.status === 'error' &&
      (result.message === 'channelBalance.denied' ||
        result.message === 'channelUsage.identity')
    ) {
      return
    }
  }
}
