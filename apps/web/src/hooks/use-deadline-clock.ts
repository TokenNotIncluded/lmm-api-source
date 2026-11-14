/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import { useCallback, useMemo, useSyncExternalStore } from 'react'

const MAX_TIMEOUT = 2_147_483_647

function subscribeToBoundary(
  notify: () => void,
  nextBoundary: (now: number) => number
) {
  let timer: number | undefined
  const refresh = () => {
    window.clearTimeout(timer)
    notify()
    const now = Date.now()
    const next = nextBoundary(now)
    if (Number.isFinite(next)) {
      timer = window.setTimeout(
        refresh,
        Math.min(Math.max(1, next - now), MAX_TIMEOUT)
      )
    }
  }
  refresh()
  window.addEventListener('focus', refresh)
  document.addEventListener('visibilitychange', refresh)
  return () => {
    window.clearTimeout(timer)
    window.removeEventListener('focus', refresh)
    document.removeEventListener('visibilitychange', refresh)
  }
}

/** Latest crossed deadline in milliseconds, or -Infinity before all deadlines.
 * This is a stable boundary snapshot, not a wall clock for request timestamps.
 */
export function useDeadlineClock(
  deadlines: readonly number[],
  serverBoundary?: number
): number {
  const key = deadlines
    .filter(Number.isFinite)
    .sort((a, b) => a - b)
    .join(',')
  const boundaries = useMemo(
    () => (key === '' ? [] : key.split(',').map(Number)),
    [key]
  )
  const getSnapshot = useCallback(() => {
    const now = Date.now()
    let crossed = -Infinity
    for (const deadline of boundaries) {
      if (deadline > now) break
      crossed = deadline
    }
    return crossed
  }, [boundaries])
  const subscribe = useCallback(
    (notify: () => void) =>
      subscribeToBoundary(
        notify,
        (now) => boundaries.find((deadline) => deadline > now) ?? Infinity
      ),
    [boundaries]
  )
  const getServerSnapshot = useCallback(
    () => serverBoundary ?? getSnapshot(),
    [serverBoundary, getSnapshot]
  )
  return useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot)
}

function localDaySnapshot() {
  const date = new Date(Date.now())
  return date.setHours(0, 0, 0, 0)
}
function subscribeLocalDay(notify: () => void) {
  return subscribeToBoundary(notify, (now) => {
    const next = new Date(now)
    next.setHours(24, 0, 0, 0)
    return next.getTime()
  })
}
export function useLocalDay(): number {
  return useSyncExternalStore(
    subscribeLocalDay,
    localDaySnapshot,
    localDaySnapshot
  )
}

function minuteSnapshot() {
  return Math.floor(Date.now() / 60_000) * 60_000
}
function subscribeMinute(notify: () => void) {
  return subscribeToBoundary(
    notify,
    (now) => (Math.floor(now / 60_000) + 1) * 60_000
  )
}
export function useCurrentMinute(): number {
  return useSyncExternalStore(subscribeMinute, minuteSnapshot, minuteSnapshot)
}
