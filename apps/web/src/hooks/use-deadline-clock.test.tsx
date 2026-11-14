/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, test } from 'node:test'

import { Window } from 'happy-dom'

const dom = new Window()
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'Node',
  'Element',
  'Event',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: dom[key],
  })
}
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { useDeadlineClock, useCurrentMinute, useLocalDay } =
  await import('./use-deadline-clock')
after(() => dom.close())

async function withClock(
  run: (clock: {
    now: () => number
    setNow: (value: number) => void
    advance: (value: number) => Promise<void>
    timers: Map<number, { due: number; run: () => void }>
  }) => Promise<void>
) {
  let now = new Date(2026, 10, 14, 12, 0, 0).getTime()
  let serial = 0
  const timers = new Map<number, { due: number; run: () => void }>()
  const realNow = Date.now
  const realTimeout = window.setTimeout
  const realClear = window.clearTimeout
  Date.now = () => now
  window.setTimeout = ((handler: TimerHandler, delay = 0) => {
    assert.equal(typeof handler, 'function')
    const id = ++serial
    timers.set(id, { due: now + delay, run: handler as () => void })
    return id
  }) as typeof window.setTimeout
  window.clearTimeout = ((id: number) => {
    timers.delete(id)
  }) as typeof window.clearTimeout
  try {
    await run({
      now: () => now,
      setNow: (value) => {
        now = value
      },
      advance: async (value) => {
        now = value
        await act(async () => {
          for (const [id, timer] of timers) {
            if (timer.due <= now) {
              timers.delete(id)
              timer.run()
            }
          }
        })
      },
      timers,
    })
  } finally {
    Date.now = realNow
    window.setTimeout = realTimeout
    window.clearTimeout = realClear
  }
}

test('a mounted control becomes active and then expired exactly at its two boundaries; replacing deadlines cancels the old wakeup', async () => {
  await withClock(async (clock) => {
    const host = document.createElement('div')
    const root = createRoot(host)
    const start = clock.now() + 1000
    const end = start + 1000
    function Control({ startAt, endAt }: { startAt: number; endAt: number }) {
      const now = useDeadlineClock([startAt, endAt])
      return (
        <button type='button' disabled={now < startAt || now >= endAt}>
          {now >= endAt ? 'expired' : now < startAt ? 'waiting' : 'active'}
        </button>
      )
    }
    try {
      await act(async () =>
        root.render(<Control startAt={start} endAt={end} />)
      )
      assert.equal(host.textContent, 'waiting')
      await clock.advance(start - 1)
      assert.equal(host.querySelector('button')?.disabled, true)
      await clock.advance(start)
      assert.equal(host.textContent, 'active')
      assert.equal(host.querySelector('button')?.disabled, false)
      await clock.advance(end)
      assert.equal(host.textContent, 'expired')
      assert.equal(host.querySelector('button')?.disabled, true)
      const next = end + 10000
      await act(async () =>
        root.render(<Control startAt={next} endAt={next + 1000} />)
      )
      assert.equal(clock.timers.size, 1)
      assert.equal([...clock.timers.values()][0]?.due, next)
    } finally {
      await act(async () => root.unmount())
    }
    assert.equal(clock.timers.size, 0)
    await act(async () => window.dispatchEvent(new Event('focus')))
    assert.equal(
      clock.timers.size,
      0,
      'unmounted subscriptions must not schedule another wakeup'
    )
  })
})

test('resuming a suspended tab rechecks deadlines before interaction and clears timer/listener subscriptions on unmount', async () => {
  await withClock(async (clock) => {
    const deadline = clock.now() + 1000
    const host = document.createElement('div')
    const root = createRoot(host)
    function Control() {
      const now = useDeadlineClock([deadline])
      return (
        <button type='button' disabled={now >= deadline}>
          claim
        </button>
      )
    }
    try {
      await act(async () => root.render(<Control />))
      clock.setNow(deadline)
      await act(async () =>
        document.dispatchEvent(new Event('visibilitychange'))
      )
      assert.equal(host.querySelector('button')?.disabled, true)
    } finally {
      await act(async () => root.unmount())
    }
    clock.setNow(deadline - 500)
    await act(async () => window.dispatchEvent(new Event('focus')))
    assert.equal(clock.timers.size, 0)
  })
})

test('calendar day and relative minute change at their boundaries without unrelated renders, and both unsubscribe', async () => {
  await withClock(async (clock) => {
    const host = document.createElement('div')
    const root = createRoot(host)
    function Display() {
      const day = useLocalDay()
      const minute = useCurrentMinute()
      return <output>{`${day}:${minute}`}</output>
    }
    try {
      await act(async () => root.render(<Display />))
      const midnight = new Date(clock.now()).setHours(24, 0, 0, 0)
      await clock.advance(midnight - 1)
      const before = host.textContent
      await clock.advance(midnight)
      assert.notEqual(host.textContent, before)
      assert.equal(host.textContent, `${midnight}:${midnight}`)
      const after = host.textContent
      await clock.advance(midnight + 1)
      assert.equal(
        host.textContent,
        after,
        'snapshots stay primitive and stable between boundaries'
      )
    } finally {
      await act(async () => root.unmount())
    }
    assert.equal(clock.timers.size, 0)
    await act(async () => document.dispatchEvent(new Event('visibilitychange')))
    assert.equal(clock.timers.size, 0)
  })
})
