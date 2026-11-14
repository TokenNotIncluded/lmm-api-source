/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */

/** The API bounds include both seconds. Adjacent buckets must never share one. */
export function buildUsageTrendBuckets(startMs: number, endMs: number) {
  const start = Math.floor(startMs / 1000)
  const end = Math.floor(endMs / 1000)
  if (!Number.isFinite(start) || !Number.isFinite(end) || end < start) return []
  const seconds = end - start + 1
  const count = Math.min(12, seconds)
  return Array.from({ length: count }, (_, index) => ({
    start: (start + Math.floor((index * seconds) / count)) * 1000,
    end: (start + Math.floor(((index + 1) * seconds) / count) - 1) * 1000,
  }))
}
