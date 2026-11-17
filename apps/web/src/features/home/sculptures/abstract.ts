/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import {
  Shape,
  TAU,
  hash,
  palette as C,
  rotate,
  tint,
  type Sculpture,
  type Vec3,
} from './geometry'

// Quiet materials keep the silhouette legible; depth supplies the contrast.
const material = {
  ivory: C.cream,
  stone: [166, 177, 172] as Vec3,
  clay: [197, 145, 119] as Vec3,
}

function stippleSurface(
  shape: Shape,
  rows: number,
  columns: number,
  sample: (u: number, v: number) => Vec3,
  color: (u: number, v: number) => Vec3
) {
  shape.surface(
    rows,
    columns,
    (u, v) => {
      const seed =
        Math.round(u * (rows - 1)) * columns + Math.round(v * (columns - 1))
      return sample(
        u === 0 || u === 1 ? u : u + ((hash(seed) - 0.5) * 0.9) / (rows - 1),
        v === 0 || v === 1
          ? v
          : v + ((hash(seed + 0.5) - 0.5) * 0.9) / (columns - 1)
      )
    },
    color
  )
}

function cage(s: Shape, radius: number, color: Vec3, part = 0) {
  const phi = (1 + Math.sqrt(5)) / 2,
    norm = Math.hypot(1, phi)
  const vertices: Vec3[] = []
  for (const a of [-1, 1]) {
    for (const b of [-phi, phi]) {
      vertices.push([0, a, b], [a, b, 0], [b, 0, a])
    }
  }
  const points = vertices.map((p) => p.map((v) => (v / norm) * radius) as Vec3)
  points.forEach((p, index) => {
    s.ellipsoid(p, [0.028, 0.028, 0.028], C.cream, part, 80)
    for (let j = index + 1; j < points.length; j++) {
      const q = points[j]
      if (Math.hypot(p[0] - q[0], p[1] - q[1], p[2] - q[2]) < radius * 1.1) {
        s.line(p, q, 0.01, color, part, 20)
      }
    }
  })
}

export function socket(): Sculpture {
  const s = new Shape()
  for (const [radius, z] of [
    [0.84, -0.12],
    [0.69, 0.15],
    [0.37, 0.24],
  ]) {
    s.ring(
      [0, 0, z],
      radius,
      radius > 0.5 ? 0.022 : 0.016,
      radius > 0.7 ? material.stone : material.ivory
    )
  }
  for (let rib = 0; rib < 12; rib++) {
    const angle = (rib / 12) * TAU
    s.tube(
      (t) => {
        const a = angle + Math.sin(t * Math.PI) * 0.23,
          r = 0.38 + t * 0.76
        return [
          Math.cos(a) * r,
          Math.sin(a) * r,
          0.19 - t * 0.3 + Math.sin(t * Math.PI) * 0.13,
        ]
      },
      0.014,
      material.stone,
      0,
      60,
      8
    )
    s.tube(
      (t) => {
        const a = angle + (t - 0.5) * 0.14
        return [Math.cos(a) * 1.14, Math.sin(a) * 1.14, -0.11]
      },
      0.019,
      material.clay,
      0,
      20,
      8
    )
    s.ellipsoid(
      [Math.cos(angle) * 0.7, Math.sin(angle) * 0.7, 0.19],
      [0.019, 0.019, 0.019],
      material.ivory,
      0,
      100
    )
  }
  cage(s, 0.25, material.stone, 1)
  s.ellipsoid([0, 0, 0], [0.095, 0.095, 0.095], material.clay, 1, 700)
  return s.model(
    (t) => {
      const wheel = rotate(2, t * 0.16),
        core = rotate(1, -t * 0.43)
      return (p, v) => {
        if (p.part) {
          core(p, v)
        }
        wheel(p, v)
      }
    },
    [0.13, 0.23]
  )
}

export function gyroscope(): Sculpture {
  const s = new Shape()
  for (const axis of [0, 1, 2] as const) {
    s.ring(
      [0, 0, 0],
      1.08 - axis * 0.23,
      0.012 + axis * 0.003,
      [material.stone, material.ivory, material.clay][axis],
      axis + 1,
      axis
    )
  }
  // A single rotor and spindle leave the nested gimbals easy to read.
  s.ellipsoid([0, 0, 0], [0.29, 0.09, 0.29], material.ivory, 4, 2600)
  s.ring([0, 0, 0], 0.29, 0.012, material.clay, 4, 1)
  s.line([0, -0.3, 0], [0, 0.3, 0], 0.011, material.stone, 4, 46)
  for (const side of [-1, 1]) {
    s.ellipsoid(
      [0, side * 0.3, 0],
      [0.018, 0.018, 0.018],
      material.ivory,
      4,
      100
    )
  }
  return s.model(
    (t) => {
      const turns = [
        rotate(1, t * 0.19),
        rotate(2, -t * 0.16),
        rotate(0, t * 0.13),
        rotate(0, 0.35 + t * 0.11),
      ]
      return (p, v) => {
        if (p.part) {
          turns[p.part - 1](p, v)
        }
      }
    },
    [0.28, 0.32]
  )
}

export function mobius(): Sculpture {
  const s = new Shape()
  const strip = (u: number, v: number): Vec3 => {
    const a = u * TAU,
      w = (v - 0.5) * 0.58,
      r = 0.8 + Math.cos(a / 2) * w
    return [Math.cos(a) * r, Math.sin(a) * r, Math.sin(a / 2) * w]
  }
  stippleSurface(s, 240, 36, strip, (u, v) =>
    tint(
      material.stone,
      0.7 + Math.sin(u * Math.PI) * 0.18 + Math.sin(v * Math.PI) * 0.12
    )
  )
  for (const v of [0, 1]) {
    s.tube((u) => strip(u, v), 0.006, material.ivory, 0, 360, 5)
  }
  return s.model((t) => rotate(1, t * 0.18), [0.1, 0.42])
}

export function trefoil(): Sculpture {
  const s = new Shape()
  const strand = (t: number, phase: number): Vec3 => {
    const a = t * TAU,
      r = (2 + Math.cos(3 * a)) * 0.32,
      dr = -0.96 * Math.sin(3 * a)
    const x = Math.cos(2 * a),
      y = Math.sin(2 * a)
    let tx = dr * x - 2 * r * y,
      ty = dr * y + 2 * r * x,
      tz = 0.96 * Math.cos(3 * a)
    const length = Math.hypot(tx, ty, tz)
    tx /= length
    ty /= length
    tz /= length
    let bx = -tz * y,
      by = tz * x,
      bz = tx * y - ty * x
    const norm = Math.hypot(bx, by, bz)
    bx /= norm
    by /= norm
    bz /= norm
    const nx = by * tz - bz * ty,
      ny = bz * tx - bx * tz,
      nz = bx * ty - by * tx
    const c = Math.cos(a * 5 + phase) * 0.045,
      d = Math.sin(a * 5 + phase) * 0.045
    return [
      x * r + nx * c + bx * d,
      y * r + ny * c + by * d,
      Math.sin(3 * a) * 0.32 + nz * c + bz * d,
    ]
  }
  for (let band = 0; band < 3; band++) {
    s.tube(
      (t) => strand(t, (band / 3) * TAU),
      0.032,
      [material.stone, material.ivory, material.clay][band],
      0,
      520,
      12
    )
  }
  return s.model(
    (t) => {
      const turn = rotate(1, t * 0.18),
        roll = rotate(2, t * 0.05)
      return (p, v) => {
        turn(p, v)
        roll(p, v)
      }
    },
    [0, 0.14]
  )
}

export function helix(): Sculpture {
  const s = new Shape()
  const rail = (u: number, phase: number): Vec3 => [
    Math.cos(u * TAU * 1.8 + phase) * 0.49,
    (u - 0.5) * 2.18,
    Math.sin(u * TAU * 1.8 + phase) * 0.49,
  ]
  for (const phase of [0, Math.PI]) {
    const color = phase ? material.clay : material.stone
    s.tube((u) => rail(u, phase), 0.033, color, 0, 240, 12)
    for (let node = 0; node < 17; node++) {
      s.ellipsoid(rail(node / 16, phase), [0.044, 0.044, 0.044], color, 0, 135)
    }
  }
  for (let i = 0; i < 17; i++) {
    const a = rail(i / 16, 0),
      b = rail(i / 16, Math.PI),
      center: Vec3 = [0, a[1], 0]
    s.line(a, center, 0.011, material.ivory, 0, 24)
    s.line(center, b, 0.011, material.ivory, 0, 24)
  }
  for (let side = 0; side < 2; side++) {
    s.ellipsoid([0, 0, 0], [0.052, 0.052, 0.052], material.ivory, side + 1, 260)
  }
  return s.model(
    (t) => {
      const turn = rotate(1, t * 0.25),
        lights = [
          rail((1 + Math.sin(t * 0.6)) / 2, 0),
          rail((1 + Math.sin(t * 0.6 + Math.PI)) / 2, Math.PI),
        ]
      return (p, v) => {
        if (p.part) {
          for (let i = 0; i < 3; i++) {
            v[i] += lights[p.part - 1][i]
          }
        }
        turn(p, v)
        for (let i = 0; i < 3; i++) {
          v[i] *= 0.9
        }
      }
    },
    [0.05, 0.12]
  )
}

export function ribbon(): Sculpture {
  const s = new Shape()
  for (let band = 0; band < 3; band++) {
    const strip = (u: number, v: number): Vec3 => {
      const a = u * TAU,
        phase = (band / 3) * TAU,
        r = 0.8 + Math.cos(a * 3 + phase) * 0.13 + (v - 0.5) * 0.2
      return [
        Math.cos(a) * r,
        Math.sin(a * 3 + phase) * 0.31 + (band - 1) * 0.13,
        Math.sin(a) * r,
      ]
    }
    stippleSurface(s, 170, 27, strip, (u, v) =>
      tint(
        [material.ivory, material.clay, material.stone][band],
        0.65 + Math.sin(v * Math.PI) * 0.22 + Math.sin(u * Math.PI) * 0.1
      )
    )
    for (const v of [0, 1]) {
      s.tube((u) => strip(u, v), 0.005, material.ivory, 0, 170, 5)
    }
  }
  return s.model(
    (t) => {
      const turn = rotate(1, t * 0.12)
      return (p, v) => {
        v[1] += Math.sin(t * 1.1 + p.x * 3 + p.z * 2) * 0.045
        turn(p, v)
      }
    },
    [0.05, 0.7]
  )
}
