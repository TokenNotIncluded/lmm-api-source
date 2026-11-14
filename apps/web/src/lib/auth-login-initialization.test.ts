/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import {
  ensureLoginInitialization,
  formatLoginIntentVersion,
  invalidateLoginIntent,
  isLoginIntentVersionCurrent,
  parseLoginIntentVersion,
  readLoginInitialization,
  readLoginIntent,
} from './auth-login-intent'

const C = 'new-api:auth-login-intent',
  I = 'new-api:auth-login-initialization'
function storage() {
  const map = new Map<string, string>(),
    writes: Array<[string, string]> = []
  return {
    map,
    writes,
    getItem: (key: string) => map.get(key) ?? null,
    setItem: (key: string, value: string) => {
      writes.push([key, value])
      map.set(key, value)
    },
  }
}
test('I initializes once without writing C and accepts initial only beside a real I', () => {
  const s = storage()
  assert.equal(readLoginIntent(s), 'initial')
  assert.equal(readLoginInitialization(s), null)
  const i = ensureLoginInitialization(s)
  assert.equal(ensureLoginInitialization(s), i)
  assert.deepEqual(s.writes, [[I, i]])
  assert.equal(s.map.has(C), false)
  const v = formatLoginIntentVersion(i, 'initial')
  assert.ok(v.length <= 128)
  assert.equal(isLoginIntentVersionCurrent(v, s), true)
})
test('explicit cancellation retires the initialization epoch without letting C deletion restore it', () => {
  const s = storage(),
    i = ensureLoginInitialization(s),
    v = formatLoginIntentVersion(i, readLoginIntent(s))
  invalidateLoginIntent(s)
  const nextInitialization = readLoginInitialization(s)
  assert.ok(nextInitialization)
  assert.notEqual(nextInitialization, i)
  assert.equal(isLoginIntentVersionCurrent(v, s), false)
  const next = formatLoginIntentVersion(nextInitialization, readLoginIntent(s))
  assert.equal(next.length, 78)
  assert.equal(isLoginIntentVersionCurrent(next, s), true)
  s.map.delete(C)
  assert.equal(isLoginIntentVersionCurrent(v, s), false)
  assert.equal(isLoginIntentVersionCurrent(next, s), false)
})
test('legacy cancellation before initialization does not create an initialization epoch', () => {
  const s = storage()
  invalidateLoginIntent(s)
  assert.equal(readLoginInitialization(s), null)
  assert.deepEqual(
    s.writes.map(([key]) => key),
    [C]
  )
})
test('failed epoch retirement rejects cancellation before reporting success', () => {
  const s = storage()
  const i = ensureLoginInitialization(s)
  const failed = {
    getItem: s.getItem,
    setItem: (key: string, value: string) => {
      if (key !== I) s.setItem(key, value)
    },
  }
  assert.throws(() => invalidateLoginIntent(failed), /storage unavailable/)
  assert.equal(readLoginInitialization(s), i)
  assert.equal(s.map.has(C), false)
})
test('missing I, raw initial/UUID, absent C, malformed or oversized redirect versions never validate', () => {
  const s = storage(),
    i = ensureLoginInitialization(s),
    v = formatLoginIntentVersion(i, 'initial')
  for (const bad of [
    undefined,
    null,
    'initial',
    i,
    `lmi1::initial`,
    `lmi1:${i}:`,
    `lmi1:${i}:bad`,
    `lmi1:${i}:initial:extra`,
    'x'.repeat(129),
  ]) {
    assert.equal(parseLoginIntentVersion(bad), null)
    assert.equal(isLoginIntentVersionCurrent(bad, s), false)
  }
  s.map.delete(I)
  assert.equal(isLoginIntentVersionCurrent(v, s), false)
  assert.equal(s.map.has(I), false)
})
test('clear and subsequent initialization cannot resurrect an issued initial-C ceremony', () => {
  const s = storage(),
    v = formatLoginIntentVersion(ensureLoginInitialization(s), 'initial')
  s.map.clear()
  assert.equal(isLoginIntentVersionCurrent(v, s), false)
  ensureLoginInitialization(s)
  assert.equal(isLoginIntentVersionCurrent(v, s), false)
})
test('I write/readback and malformed stored initialization fail closed', () => {
  const s = storage()
  const failed = { getItem: s.getItem, setItem: () => {} }
  assert.throws(() => ensureLoginInitialization(failed), /storage unavailable/)
  assert.equal(s.map.has(C), false)
  s.map.set(I, 'initial')
  assert.throws(() => readLoginInitialization(s), /storage unavailable/)
})
