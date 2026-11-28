/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import type { UsageLog } from '../data/schema'
import type { LogOtherData } from '../types'

/** Native timings retain their millisecond precision; missing samples stay unknown. */
export function getLogTiming(
  log: Pick<
    UsageLog,
    | 'duration_ms'
    | 'first_token_ms'
    | 'use_time'
    | 'completion_tokens'
    | 'state'
  >,
  other: Pick<LogOtherData, 'frt'> | null
) {
  const native = log.state !== undefined
  const durationMs =
    log.duration_ms ??
    (!native && log.use_time != null && log.use_time > 0
      ? log.use_time * 1000
      : null)
  const firstTokenMs =
    log.first_token_ms ??
    (!native && other?.frt != null && other.frt > 0 ? other.frt : null)
  const outputTokens = log.completion_tokens
  const throughput =
    durationMs !== null &&
    durationMs > 0 &&
    outputTokens !== null &&
    outputTokens >= 0
      ? (outputTokens * 1000) / durationMs
      : null
  return { durationMs, firstTokenMs, throughput }
}

export function formatDurationMs(value: number | null | undefined): string {
  if (value == null) return '—'
  return value < 1000
    ? `${Number(value.toFixed(2))} ms`
    : `${Number((value / 1000).toFixed(3))} s`
}
