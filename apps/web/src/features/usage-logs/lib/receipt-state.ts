/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import type { UsageLog } from '../data/schema'
import type { LogOtherData } from '../types'

type ReceiptStateLog = Pick<UsageLog, 'billing_basis' | 'state'>
type ReceiptStateMetadata = Pick<
  LogOtherData,
  'billing_basis' | 'request_state'
>
type IncompleteReceiptState =
  | { status: 'reconcile'; label: 'Reconciling'; variant: 'warning' }
  | { status: 'incomplete'; label: 'Incomplete'; variant: 'warning' }

/** Native incomplete receipts retain their state without changing legacy log types. */
export function usageReceiptState(
  log: ReceiptStateLog,
  other: ReceiptStateMetadata | null = null
): IncompleteReceiptState | undefined {
  if (
    log.billing_basis !== 'incomplete' &&
    other?.billing_basis !== 'incomplete'
  ) {
    return undefined
  }
  return (log.state ?? other?.request_state) === 'reconcile'
    ? { status: 'reconcile', label: 'Reconciling', variant: 'warning' }
    : { status: 'incomplete', label: 'Incomplete', variant: 'warning' }
}

export function missingFinalUsageReceipt(
  log: ReceiptStateLog & Pick<UsageLog, 'outcome'>,
  other: ReceiptStateMetadata | null
): boolean {
  return log.outcome === null && usageReceiptState(log, other) !== undefined
}

export function usageLogStatusCode(
  log: Pick<UsageLog, 'status_code' | 'provider_http_status'>,
  other: Pick<LogOtherData, 'status_code'> | null
): number | undefined {
  const status =
    log.status_code ?? log.provider_http_status ?? other?.status_code
  return status != null &&
    Number.isInteger(status) &&
    status >= 100 &&
    status <= 599
    ? status
    : undefined
}

export const requestResultLabels = {
  success: 'Success',
  error: 'Error',
  disconnected: 'Disconnected or cancelled',
} as const

export const settlementStatusLabels = {
  pending: 'Awaiting usage receipt',
  settled: 'Settled',
  released: 'Released',
  refunded: 'Refunded',
} as const

export function nativeRequestStates(log: UsageLog, other: LogOtherData | null) {
  const state = log.state ?? other?.request_state
  if (!state && !log.settlement_status) return
  const settlement: UsageLog['settlement_status'] =
    log.settlement_status ??
    (state === 'settled'
      ? (log.refunded_credits ?? 0) > 0
        ? 'refunded'
        : 'settled'
      : state === 'released'
        ? 'released'
        : state === 'reserved' || state === 'streaming' || state === 'reconcile'
          ? 'pending'
          : undefined)
  const status = usageLogStatusCode(log, other)
  // Older Core versions project the recording fields without the two new labels.
  const result: UsageLog['request_result'] =
    log.request_result !== undefined
      ? log.request_result
      : log.outcome === 'completed'
        ? 'success'
        : log.outcome === 'cancelled'
          ? 'disconnected'
          : log.outcome === 'failed' || (status !== undefined && status >= 400)
            ? 'error'
            : null
  return { result, settlement }
}
