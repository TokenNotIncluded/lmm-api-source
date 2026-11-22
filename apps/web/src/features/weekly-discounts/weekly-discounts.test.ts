/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import test from 'node:test'

import {
  createWeeklyClient,
  parseWeeklyHistory,
  parseWeeklyInspection,
  weeklyAPIPath,
  WeeklyRequestFailure,
} from './api'
import { WeeklyDetailSession } from './detail-session'

const id = 'a'.repeat(64)
const week = 345_600 + 2_900 * 604_800
const summary = {
  id,
  week_start: week,
  expires_at: week + 604_800,
  status: 'offered',
  discount_percent: 10,
  created_at: week + 60,
  claimed_at: null,
  browser_path: weeklyAPIPath + '/' + id,
}
const decision = { ...summary, reason: 'A complete, original decision reason.' }
const offered = { ok: true, decision, claimable: true, csrf: '1'.repeat(64) }
const privateCode = 'AIWEEK-PRIVATE-BROWSER-FIXTURE'
const claimedDecision = {
  ...decision,
  status: 'claimed',
  claimed_at: week + 120,
  code: privateCode,
}
const claimed = { ok: true, decision: claimedDecision, executed: true }
const inspectedClaimed = {
  ok: true,
  decision: claimedDecision,
  claimable: false,
  csrf: '2'.repeat(64),
}
type Step = unknown | (() => Promise<Response>)
function json(value: unknown) {
  return new Response(JSON.stringify(value), {
    headers: { 'Content-Type': 'application/json' },
  })
}
function transport(steps: Step[]) {
  const calls: { path: string; init?: RequestInit }[] = []
  const fetcher = (async (input: RequestInfo | URL, init?: RequestInit) => {
    calls.push({ path: String(input), init })
    const step = steps.shift()
    if (typeof step === 'function') {
      return await step()
    }
    if (step === undefined) {
      throw new Error('Unexpected request')
    }
    return json(step)
  }) as typeof fetch
  return { calls, client: createWeeklyClient(fetcher) }
}
function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => {
    resolve = done
  })
  return { promise, resolve }
}

test('history uses a server UTC week and contains only closed public summaries', () => {
  const value = {
    ok: true,
    current_utc_week: week,
    decisions: [summary],
    next_before_week: null,
  }
  assert.equal(parseWeeklyHistory(value).current_utc_week, week)
  for (const changed of [
    { ...value, current_utc_week: week + 1 },
    { ...value, decisions: [{ ...summary, code: privateCode }] },
    { ...value, decisions: [summary, summary] },
    {
      ...value,
      decisions: [
        {
          ...summary,
          week_start: week + 604_800,
          expires_at: week + 1_209_600,
        },
      ],
    },
    { ...value, next_before_week: week },
  ]) {
    assert.throws(() => parseWeeklyHistory(changed), WeeklyRequestFailure)
  }
})

test('detail binds the original id, exact API path, complete reason and code visibility', () => {
  assert.equal(
    parseWeeklyInspection(offered, id).decision.reason,
    decision.reason
  )
  assert.equal(
    parseWeeklyInspection(inspectedClaimed, id).decision.code,
    privateCode
  )
  for (const changed of [
    { ...offered, decision: { ...decision, id: 'b'.repeat(64) } },
    {
      ...offered,
      decision: { ...decision, browser_path: 'https://other.invalid/' + id },
    },
    {
      ...offered,
      decision: {
        ...decision,
        browser_path: weeklyAPIPath + '/../weekly-discounts/' + id,
      },
    },
    {
      ...offered,
      decision: {
        ...decision,
        browser_path: weeklyAPIPath + '/' + id + '?claim=true',
      },
    },
    {
      ...offered,
      decision: { ...decision, undisclosed_term: 'extra restriction' },
    },
    { ...offered, decision: { ...decision, code: privateCode } },
    { ...offered, csrf: undefined },
  ]) {
    assert.throws(
      () => parseWeeklyInspection(changed, id),
      WeeklyRequestFailure
    )
  }
})

test('history pagination is exact, descending and cannot revive a previous week', async () => {
  const rows = Array.from({ length: 100 }, (_, index) => {
    const rowID = index.toString(16).padStart(64, '0')
    const rowWeek = week - index * 604_800
    return {
      ...summary,
      id: rowID,
      week_start: rowWeek,
      expires_at: rowWeek + 604_800,
      created_at: rowWeek + 60,
      browser_path: weeklyAPIPath + '/' + rowID,
    }
  })
  const page = {
    ok: true,
    current_utc_week: week,
    decisions: rows,
    next_before_week: rows[99].week_start,
  }
  assert.equal(parseWeeklyHistory(page).decisions.length, 100)
  assert.throws(
    () =>
      parseWeeklyHistory({
        ...page,
        decisions: page.decisions.map((row, index) =>
          index === 1 ? { ...row, browser_path: summary.browser_path } : row
        ),
      }),
    WeeklyRequestFailure
  )
  const { client, calls } = transport([page])
  await assert.rejects(
    client.history(new AbortController().signal, week),
    WeeklyRequestFailure
  )
  assert.equal(calls[0].path, weeklyAPIPath + '?before_week=' + String(week))
})

test('only exact ids reach the fixed same-origin JSON endpoint', async () => {
  const { client, calls } = transport([offered])
  for (const invalid of [
    id + '?claim=true',
    '../' + id,
    id.toUpperCase(),
    '%61' + id.slice(1),
    '//other.invalid',
  ]) {
    await assert.rejects(
      client.inspect(invalid, new AbortController().signal),
      WeeklyRequestFailure
    )
  }
  assert.equal(calls.length, 0)
  await client.inspect(id, new AbortController().signal)
  const init = calls[0].init!
  assert.equal(calls[0].path, weeklyAPIPath + '/' + id)
  assert.equal(init.method, 'GET')
  assert.equal(init.credentials, 'same-origin')
  assert.equal(init.mode, 'same-origin')
  assert.equal(init.cache, 'no-store')
  assert.equal(init.redirect, 'error')
  assert.equal(init.referrerPolicy, 'no-referrer')
  assert.deepEqual(init.headers, { Accept: 'application/json' })
  assert.equal(init.body, undefined)
})

test('opening and querying an offer never claims it', async () => {
  const { client, calls } = transport([offered, offered])
  const session = new WeeklyDetailSession(id, client, () => true)
  await session.query()
  await session.query()
  assert.equal(session.getSnapshot().phase, 'ready')
  assert.deepEqual(
    calls.map((call) => call.init?.method),
    ['GET', 'GET']
  )
})

test('one explicit claim uses the original nonce and double activation cannot send twice', async () => {
  const waiting = deferred<Response>()
  const { client, calls } = transport([offered, () => waiting.promise])
  const session = new WeeklyDetailSession(id, client, () => true)
  await session.query()
  const first = session.claim()
  await session.claim()
  assert.equal(calls.length, 2)
  assert.equal(session.getSnapshot().phase, 'claiming')
  assert.equal(session.getSnapshot().inspection?.csrf, undefined)
  assert.equal(calls[1].path, weeklyAPIPath + '/' + id)
  assert.equal(calls[1].init?.body, '{"claim":true}')
  assert.deepEqual(calls[1].init?.headers, {
    Accept: 'application/json',
    'Content-Type': 'application/json',
    'x-lmm-assistant-csrf': offered.csrf,
  })
  waiting.resolve(json(claimed))
  await first
  assert.equal(session.getSnapshot().inspection?.decision.code, privateCode)
  await session.claim()
  assert.equal(calls.length, 2)
})

test('a lost claim response permits only querying the original decision', async () => {
  const { client, calls } = transport([
    offered,
    async () => {
      throw new Error('Synthetic lost acknowledgement')
    },
    async () => new Response('Unavailable', { status: 503 }),
    inspectedClaimed,
  ])
  const session = new WeeklyDetailSession(id, client, () => true)
  await session.query()
  await session.claim()
  assert.equal(session.getSnapshot().phase, 'unknown')
  await session.claim()
  assert.equal(calls.length, 2)
  await session.query()
  assert.equal(session.getSnapshot().phase, 'unknown')
  await session.claim()
  assert.equal(calls.length, 3)
  await session.query()
  assert.equal(session.getSnapshot().inspection?.decision.code, privateCode)
  assert.deepEqual(
    calls.map((call) => call.init?.method),
    ['GET', 'POST', 'GET', 'GET']
  )
  assert.ok(calls.every((call) => call.path === weeklyAPIPath + '/' + id))
})

test('fresh original readback may enable a new human claim but never submits it', async () => {
  const newer = { ...offered, csrf: '3'.repeat(64) }
  const { client, calls } = transport([
    offered,
    async () =>
      new Response('', { headers: { 'Content-Type': 'application/json' } }),
    newer,
    claimed,
  ])
  const session = new WeeklyDetailSession(id, client, () => true)
  await session.query()
  await session.claim()
  await session.query()
  assert.deepEqual(
    calls.map((call) => call.init?.method),
    ['GET', 'POST', 'GET']
  )
  await session.claim()
  assert.equal(
    new Headers(calls[3].init?.headers).get('x-lmm-assistant-csrf'),
    newer.csrf
  )
})

test('a cleared or different-session view cannot publish a late private GET response', async () => {
  const waiting = deferred<Response>()
  let current = true
  const { client, calls } = transport([() => waiting.promise])
  const session = new WeeklyDetailSession(id, client, () => current)
  const read = session.query()
  current = false
  session.clear()
  waiting.resolve(json(inspectedClaimed))
  await read
  assert.equal(calls[0].init?.signal?.aborted, true)
  assert.deepEqual(session.getSnapshot(), {
    phase: 'cleared',
    inspection: null,
  })
  await session.query()
  await session.claim()
  assert.equal(calls.length, 1)
})

test('a late private claim response cannot cross an account or Session switch', async () => {
  const waiting = deferred<Response>()
  let current = true
  const { client } = transport([offered, () => waiting.promise])
  const session = new WeeklyDetailSession(id, client, () => current)
  await session.query()
  const claim = session.claim()
  current = false
  session.clear()
  waiting.resolve(json(claimed))
  await claim
  assert.deepEqual(session.getSnapshot(), {
    phase: 'cleared',
    inspection: null,
  })
})

test('leaving a visible claimed page destroys the private snapshot and nonce', async () => {
  const { client, calls } = transport([inspectedClaimed])
  const session = new WeeklyDetailSession(id, client, () => true)
  await session.query()
  assert.equal(session.getSnapshot().inspection?.decision.code, privateCode)
  session.clear()
  assert.deepEqual(session.getSnapshot(), {
    phase: 'cleared',
    inspection: null,
  })
  await session.claim()
  assert.equal(calls.length, 1)
})

test('expired, declined or wrong-session offers cannot submit a claim', async () => {
  const { client, calls } = transport([
    { ...offered, claimable: false, csrf: undefined },
  ])
  const session = new WeeklyDetailSession(id, client, () => true)
  await session.query()
  await session.claim()
  assert.deepEqual(
    calls.map((call) => call.init?.method),
    ['GET']
  )
})

test('a changed receipt becomes unknown and errors never contain the private response', async () => {
  const { client, calls } = transport([
    offered,
    {
      ...claimed,
      decision: { ...claimedDecision, discount_percent: 11 },
    },
  ])
  const session = new WeeklyDetailSession(id, client, () => true)
  await session.query()
  await session.claim()
  assert.equal(session.getSnapshot().phase, 'unknown')
  assert.equal(session.getSnapshot().inspection?.decision.code, undefined)
  await session.claim()
  assert.equal(calls.length, 2)
  const bad = transport([{ ok: false, code: privateCode, error: privateCode }])
  await assert.rejects(
    bad.client.inspect(id, new AbortController().signal),
    (error: unknown) => {
      assert.ok(error instanceof WeeklyRequestFailure)
      assert.ok(!error.message.includes(privateCode))
      return true
    }
  )
})
