/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import {
  newLights,
  newSlide,
  slideTile,
  solvedSlide,
  toggleLights,
} from './engine'

test('sliding puzzle moves only adjacent tiles and recognizes completion', () => {
  const board = [1, 2, 3, 4, 5, 6, 7, 0, 8]
  assert.equal(slideTile(board, 0), board)
  assert.equal(solvedSlide(slideTile(board, 8)), true)
  assert.deepEqual(board, [1, 2, 3, 4, 5, 6, 7, 0, 8])
})
test('generated sliding boards are unsolved and solvable', () => {
  for (let run = 0; run < 40; run++) {
    const board = newSlide()
    assert.equal(solvedSlide(board), false)
    assert.deepEqual([...board].sort(), [0, 1, 2, 3, 4, 5, 6, 7, 8])
    const tiles = board.filter(Boolean)
    const inversions = tiles.reduce(
      (sum, tile, i) =>
        sum + tiles.slice(i + 1).filter((other) => other < tile).length,
      0
    )
    assert.equal(inversions % 2, 0)
  }
})
test('lights toggle orthogonal neighbors without wrapping and are reversible', () => {
  const off = Array<boolean>(16).fill(false)
  assert.deepEqual(
    toggleLights(off, 3).flatMap((on, i) => (on ? [i] : [])),
    [2, 3, 7]
  )
  assert.deepEqual(toggleLights(toggleLights(off, 5), 5), off)
  for (let run = 0; run < 5; run++) assert.ok(newLights().some(Boolean))
})
