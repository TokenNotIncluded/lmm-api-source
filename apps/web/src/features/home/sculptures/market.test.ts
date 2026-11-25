/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import type { Point, Sculpture, Vec3 } from './geometry'
import { shop, stall, vending } from './market'

function at(model: Sculpture, p: Point, time: number) {
  const v: Vec3 = [p.x, p.y, p.z]
  model.animate(time)(p, v)
  return v
}

test('complete shop models fit the mobile budget throughout their motion', () => {
  for (const build of [shop, vending, stall]) {
    const model = build()
    assert.ok(model.points.length <= 22000, build.name)
    for (const time of [0, 1.4, 2.5, 4.2, 5.6]) {
      const pose = model.animate(time)
      for (const p of model.points) {
        const rest: Vec3 = [p.x, p.y, p.z],
          v: Vec3 = [...rest]
        pose(p, v)
        assert.deepEqual([p.x, p.y, p.z], rest)
        assert.ok(v.every((x) => Number.isFinite(x) && Math.abs(x) < 1.3))
        assert.ok(p.color.every((x) => x >= 0 && x <= 255))
      }
    }
  }
})

test('awnings stay pinned at the back and their loose edges move with the wind', () => {
  for (const model of [shop(), stall()]) {
    const cloth = model.points.filter((p) => p.part === 1)
    const back = Math.min(...cloth.map((p) => p.z)),
      front = Math.max(...cloth.map((p) => p.z))
    const seam = cloth.filter((p) => p.z === back),
      edge = cloth.filter((p) => p.z === front)
    assert.ok(seam.length > 10 && edge.length > 10)
    for (const p of seam) assert.deepEqual(at(model, p, 1.1), [p.x, p.y, p.z])
    assert.ok(edge.some((p) => Math.abs(at(model, p, 1.1)[1] - p.y) > 0.03))
  }
})

test('the door swings towards the street and the stall sign and ties share a pivot', () => {
  const store = shop(),
    stallModel = stall()
  const handle = store.points.find(
    (p) => p.part === 2 && p.x > 0.118 && p.z >= 0.42
  )
  assert.ok(handle)
  assert.ok(at(store, handle, Math.PI / 0.82)[2] > handle.z + 0.11)
  const pivot: Vec3 = [0.66, 0.74, 0.48]
  const distance = (a: Vec3, b: Vec3) =>
    Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2])
  for (const p of stallModel.points.filter((p) => p.part === 2)) {
    assert.ok(
      Math.abs(
        distance(at(stallModel, p, 1), pivot) - distance([p.x, p.y, p.z], pivot)
      ) < 1e-10
    )
  }
})

test('the vending bottle lands sideways and returns behind the cabinet before reappearing', () => {
  const model = vending(),
    bottle = model.points.filter((p) => p.part === 1)
  const sample = (fraction: number) =>
    bottle.map((p) => at(model, p, fraction * 5.6))
  const rest = sample(0),
    tray = sample(0.5),
    concealed = sample(0.79)
  const range = (points: Vec3[], axis: number) =>
    Math.max(...points.map((p) => p[axis])) -
    Math.min(...points.map((p) => p[axis]))
  assert.ok(range(rest, 1) > range(rest, 0) * 1.7)
  assert.ok(range(tray, 0) > range(tray, 1) * 1.5)
  assert.ok(tray.every((p) => p[1] < -0.68 && p[1] > -0.9 && p[2] > 0.49))
  assert.ok(concealed.every((p) => p[2] < 0.4))
  assert.deepEqual(sample(1), rest)
})
