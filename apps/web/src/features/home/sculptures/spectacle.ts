/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import {
  Shape,
  TAU,
  hash,
  palette as C,
  rotate,
  smooth,
  tint,
  type Sculpture,
  type Vec3,
} from './geometry'

export function spacexRocket(): Sculpture {
  const s = new Shape()
  const hull: Vec3 = [223, 231, 239]
  s.surface(
    95,
    50,
    (u, v) => [
      Math.cos(v * TAU) * 0.12,
      -0.58 + u * 0.87,
      Math.sin(v * TAU) * 0.12,
    ],
    (u, v) =>
      tint(
        u > 0.31 && u < 0.43 ? C.ink : hull,
        0.62 + Math.max(0, Math.sin(v * TAU)) * 0.38
      ),
    1
  )
  s.surface(
    42,
    44,
    (u, v) => {
      const r = 0.14 * Math.cos((u * Math.PI) / 2)
      return [Math.cos(v * TAU) * r, 0.29 + u * 0.3, Math.sin(v * TAU) * r]
    },
    (_u, v) => tint(hull, 0.64 + Math.max(0, Math.sin(v * TAU)) * 0.36),
    1
  )
  for (const y of [-0.53, -0.29, 0.26]) {
    s.ring([0, y, 0], 0.122, 0.009, C.ink, 1, 1)
  }
  for (const side of [-1, 1]) {
    s.surface(
      22,
      15,
      (u, v) => [
        side * (0.105 + u * 0.095),
        -0.5 + u * 0.16 + v * (1 - u) * 0.11,
        (v - 0.5) * 0.028,
      ],
      C.ink,
      1
    )
  }
  // A compact SpaceX wordmark drawn as strokes along the fuselage.
  const letters = [
    [
      [1, 0],
      [0, 0],
      [0, 1],
      [1, 1],
      [1, 2],
      [0, 2],
    ],
    [
      [0, 2],
      [0, 0],
      [1, 0],
      [1, 1],
      [0, 1],
    ],
    [
      [0, 2],
      [0.5, 0],
      [1, 2],
      [0.75, 1],
      [0.25, 1],
    ],
    [
      [1, 0],
      [0, 0],
      [0, 2],
      [1, 2],
    ],
    [
      [1, 0],
      [0, 0],
      [0, 1],
      [0.8, 1],
      [0, 1],
      [0, 2],
      [1, 2],
    ],
    [
      [0, 0],
      [1, 2],
      [0.5, 1],
      [1, 0],
      [0, 2],
    ],
  ]
  for (const [index, letter] of letters.entries()) {
    for (let i = 1; i < letter.length; i++) {
      const point = ([x, y]: number[]): Vec3 => [
        (y - 1) * 0.025,
        0.16 - (index * 1.5 + x) * 0.048,
        0.124,
      ]
      s.line(point(letter[i - 1]), point(letter[i]), 0.003, C.ink, 1, 6)
    }
  }
  s.surface(
    45,
    32,
    (u, v) => {
      const r = (1 - u) * (0.065 + u * 0.07)
      return [Math.cos(v * TAU) * r, -0.59 - u * 0.44, Math.sin(v * TAU) * r]
    },
    (u) => (u < 0.22 ? hull : u < 0.62 ? C.gold : C.rust),
    2
  )
  s.box([0, -0.94, 0], [0.77, 0.07, 0.56], [90, 108, 125], 0)
  s.line([-0.38, -0.91, -0.15], [-0.38, 0.28, -0.15], 0.038, [109, 131, 151])
  for (let i = 0; i < 11; i++) {
    const side = i % 2 ? 1 : -1,
      r = 0.09 + hash(i + 10) * 0.1
    s.ellipsoid(
      [
        side * (0.13 + hash(i) * 0.39),
        -0.88 + hash(i + 3) * 0.11,
        hash(i + 7) * 0.42,
      ],
      [r * 1.4, r, r],
      [177, 191, 204],
      10 + i,
      350
    )
  }
  return s.model(
    (t) => {
      const phase = ((t % 12) + 12) % 12
      const lift = smooth(Math.min(1, phase / 6)) * 0.5
      return (p, v) => {
        if (p.part === 1 || p.part === 2) {
          v[1] += lift
          if (p.part === 2) {
            const length = (-0.59 - p.y) / 0.44
            v[0] += Math.sin(t * 23 + p.y * 24) * length * 0.012
            v[1] -= length * (0.045 + Math.sin(t * 19) * 0.028)
          }
        } else if (p.part >= 10) {
          const expansion = 1 + Math.sin(t * 1.2 + p.part) * 0.15
          v[0] *= expansion
          v[1] += Math.sin(t * 1.6 + p.part) * 0.018
        }
      }
    },
    [0.16, -0.08]
  )
}

/** A terracotta plume with a fluted stem, scalloped canopy and two ripple rings. */
export function atomicExplosion(): Sculpture {
  const s = new Shape()
  const clay: Vec3 = [201, 125, 97]
  const shadow: Vec3 = [128, 90, 99]
  s.ellipsoid([0, -0.75, 0], [0.32, 0.14, 0.31], C.gold, 1, 950)
  s.surface(
    65,
    52,
    (u, v) => {
      const a = v * TAU
      const profile = 0.105 + 0.14 * (1 - u) ** 3 + 0.26 * u ** 3
      const r = profile * (1 + 0.18 * Math.cos(5 * a - 4 * u))
      return [Math.cos(a) * r, -0.77 + 1.26 * u, Math.sin(a) * r]
    },
    (u, v) =>
      tint(
        u < 0.28 ? C.gold : clay,
        0.76 + 0.22 * Math.cos(5 * TAU * v - 4 * u)
      ),
    2
  )
  s.surface(
    58,
    91,
    (u, v) => {
      const latitude = (u - 0.5) * Math.PI
      const a = v * TAU
      const edge = Math.cos(latitude)
      const r = edge * (0.74 + 0.055 * edge * Math.cos(10 * a))
      return [
        r * Math.cos(a),
        0.57 + 0.29 * Math.sin(latitude) + 0.03 * edge * Math.sin(3 * a),
        r * Math.sin(a) * 0.9,
      ]
    },
    (u, v) =>
      tint(
        u < 0.36 ? shadow : u < 0.65 ? clay : C.gold,
        0.82 + 0.16 * Math.cos(v * TAU - 1)
      ),
    3
  )
  for (let i = 0; i < 10; i++) {
    const a = (i / 10) * TAU
    const r = 0.16 + hash(i + 300) * 0.055
    s.ellipsoid(
      [Math.cos(a) * 0.59, 0.51 + Math.sin(a * 3) * 0.052, Math.sin(a) * 0.54],
      [r * 1.2, r, r],
      i % 3 === 0 ? C.gold : clay,
      3,
      410
    )
  }
  // Overlapping billows break the otherwise even upper silhouette.
  for (let i = 0; i < 6; i++) {
    const a = (i / 6) * TAU
    const lift = 0.055 * Math.sin(2 * a + 0.6)
    s.ellipsoid(
      [0.34 * Math.cos(a), 0.75 + lift, 0.26 * Math.sin(a)],
      [0.24, 0.145 + lift * 0.4, 0.21],
      i % 2 ? C.gold : clay,
      3,
      330
    )
  }
  s.ellipsoid([0.02, 0.84, -0.04], [0.26, 0.16, 0.22], C.gold, 3, 420)
  for (let i = 0; i < 12; i++) {
    const a = (i / 12) * TAU
    const spread = 0.12 + hash(i + 22) * 0.18
    s.tube(
      (t) => [
        Math.cos(a) * (0.38 + spread * t),
        -0.68 + (0.05 + spread) * Math.sin(t * Math.PI * 0.7),
        Math.sin(a) * (0.38 + spread * t),
      ],
      0.007,
      C.rust,
      5,
      14,
      4
    )
  }
  s.ring([0, -0.8, 0], 0.94, 0.015, C.gold, 4, 1)
  s.ring([0, -0.76, 0], 0.78, 0.01, clay, 5, 1)
  return s.model(
    (t) => {
      const angle = (t / 7.2) * TAU
      const rise = (1 - Math.cos(angle)) * 0.5
      const height = 0.67 + 0.33 * rise
      const bloom = 0.72 + 0.28 * (1 - Math.cos(angle - 0.48)) * 0.5
      const drift = 0.075 * Math.sin(angle)
      const turn = rotate(1, 0.18 * Math.sin(angle - 0.3))
      const outer = 0.79 - 0.21 * Math.cos(angle - 0.8)
      const inner = 0.74 - 0.25 * Math.cos(angle - 2.4)
      return (p, v) => {
        if (p.part === 2 || p.part === 3) {
          const aboveBase = p.y + 0.77
          const spread = p.part === 2 ? 0.87 + 0.13 * rise : bloom
          v[0] *= spread
          v[2] *= spread
          turn(p, v)
          v[0] += drift * aboveBase
          v[1] = -0.77 + aboveBase * height
        } else if (p.part === 4 || p.part === 5) {
          const radius = p.part === 4 ? outer : inner
          v[0] *= radius
          v[2] *= radius
        }
      }
    },
    [0.2, -0.26]
  )
}

export function rotatingChair(): Sculpture {
  const s = new Shape()
  const wood: Vec3 = [175, 112, 71]
  const fabric: Vec3 = [97, 153, 177]
  s.box([0, -0.13, 0], [0.92, 0.13, 0.79], wood)
  s.ellipsoid([0, -0.02, 0.03], [0.45, 0.12, 0.39], fabric, 0, 4600)
  s.box([0, 0.45, -0.36], [0.98, 0.86, 0.11], wood)
  s.ellipsoid([0, 0.45, -0.26], [0.435, 0.39, 0.12], fabric, 0, 4700)
  for (const x of [-0.38, 0.38]) {
    for (const z of [-0.29, 0.29]) {
      s.line([x, -0.15, z], [x * 1.2, -0.94, z * 1.26], 0.04, wood)
    }
    s.line([x * 1.27, 0.27, -0.32], [x * 1.27, 0.27, 0.36], 0.045, wood)
    s.line([x * 1.27, -0.14, 0.27], [x * 1.27, 0.27, 0.27], 0.025, wood)
  }
  for (const x of [-0.19, 0.19]) {
    s.ellipsoid(
      [x, 0.47, -0.137],
      [0.022, 0.022, 0.012],
      tint(fabric, 0.58),
      0,
      120
    )
  }
  return s.model((t) => rotate(1, t * 0.58), [0.2, -0.2])
}
