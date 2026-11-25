/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import {
  Shape,
  TAU,
  palette as C,
  rotate,
  smooth,
  tint,
  type Sculpture,
  type Vec3,
} from './geometry'

// The front and top carry the stock and signage. Sample the concealed faces
// more sparingly, leaving room for the shapes people see through the windows.
function panel(s: Shape, center: Vec3, size: Vec3, color: Vec3, part = 0) {
  for (const axis of [0, 1, 2] as const) {
    const a = (axis + 1) % 3,
      b = (axis + 2) % 3
    for (const side of [-1, 1]) {
      const density = axis === 2 && side === 1 ? 34 : side < 0 ? 15 : 23
      const count = Math.max(
        8,
        Math.ceil(size[a] * size[b] * density * density)
      )
      const shade = tint(
        color,
        axis === 2 && side === 1 ? 1 : axis === 1 ? 0.9 : 0.73
      )
      for (let i = 0; i < count; i++) {
        const p: Vec3 = [...center]
        p[axis] += (side * size[axis]) / 2
        p[a] += ((i + 0.5) / count - 0.5) * size[a]
        p[b] += (((i * 0.61803398875) % 1) - 0.5) * size[b]
        s.add(p, shade, part)
      }
    }
  }
}

function awning(s: Shape, y: number, depth: number, fabric: Vec3) {
  for (let stripe = 0; stripe < 10; stripe++) {
    const color = stripe % 2 ? C.cream : fabric
    const x = (u: number) => -1.06 + (stripe + u) * 0.212
    const crest = (u: number) => Math.sin(u * Math.PI)
    s.surface(
      11,
      24,
      (u, v) => [
        x(u),
        y + 0.1 * Math.sin(v * Math.PI) - 0.2 * v + 0.03 * crest(u) * v,
        (v - 0.5) * depth,
      ],
      (u, v) => tint(color, 0.8 + 0.14 * crest(u) + 0.06 * v),
      1
    )
    s.surface(
      13,
      8,
      (u, v) => [
        x(u),
        y - 0.2 + 0.03 * crest(u) - v * (0.095 + 0.09 * crest(u)),
        depth / 2 + 0.024 * Math.sin(Math.PI * v),
      ],
      (u, v) => tint(color, 0.72 + 0.13 * crest(u) + 0.15 * v),
      1
    )
  }
}

// Wind weights are baked into the loose fabric; the back seam stays pinned.
function clothWeights(s: Shape, depth: number) {
  for (const p of s.points) {
    if (p.part !== 1) continue
    const reach = Math.min(1, Math.max(0, p.z / depth + 0.5)) ** 2
    p.closed = [reach * Math.cos(p.x * 2.6), reach * Math.sin(p.x * 2.6), 0]
  }
}
function breeze(t: number) {
  const a = Math.sin(t * 1.7) * 0.035,
    b = Math.cos(t * 1.7) * 0.035
  return (weights: Vec3, v: Vec3) => {
    const lift = a * weights[0] + b * weights[1]
    v[1] += lift
    v[2] += lift * 0.42
  }
}

// Turn a bottle profile with a shoulder, paper label and a short metal cap.
function bottle(s: Shape, center: Vec3, color: Vec3, part = 0, height = 1) {
  const profile = [
    0.045, 0.062, 0.063, 0.063, 0.063, 0.061, 0.046, 0.029, 0.029, 0.035, 0.035,
    0,
  ]
  s.surface(
    profile.length,
    17,
    (u, v) => {
      const r = profile[Math.round(u * (profile.length - 1))],
        angle = v * TAU + u * 0.13
      return [
        center[0] + r * Math.cos(angle),
        center[1] + (u - 0.45) * 0.24 * height,
        center[2] + r * Math.sin(angle),
      ]
    },
    (u, v) =>
      tint(
        u >= 0.8 ? C.gold : u > 0.18 && u < 0.46 ? C.cream : color,
        0.73 + 0.27 * Math.max(0, Math.sin(v * TAU - 0.4))
      ),
    part
  )
}
export function shop(): Sculpture {
  const s = new Shape()
  panel(s, [0, -0.22, -0.12], [1.88, 1.43, 0.82], tint(C.teal, 0.8))
  panel(s, [0, -0.98, 0], [2.2, 0.1, 1.14], C.ink)
  for (const side of [-1, 1]) {
    panel(s, [side * 0.61, -0.23, 0.31], [0.54, 0.92, 0.03], C.ink)
    for (let row = 0; row < 2; row++) {
      const y = -0.6 + row * 0.39
      panel(s, [side * 0.61, y, 0.4], [0.5, 0.035, 0.16], C.gold)
      for (let col = 0; col < 3; col++) {
        bottle(
          s,
          [side * 0.61 + (col - 1) * 0.155, y + 0.13, 0.4],
          [C.rose, C.gold, C.teal][col],
          0,
          col === 1 ? 1.22 : 0.92
        )
      }
    }
    s.line(
      [side * 0.93, 0.19, 0.42],
      [side * 1.08, 0.19, 0.45],
      0.017,
      C.gold,
      0,
      14
    )
    s.line(
      [side * 1.08, 0.19, 0.45],
      [side * 1.08, 0.1, 0.45],
      0.01,
      C.gold,
      3,
      9
    )
    s.ellipsoid(
      [side * 1.08, 0.045, 0.45],
      [0.077, 0.083, 0.077],
      C.gold,
      3,
      190
    )
    panel(s, [side * 1.0, -0.8, 0.47], [0.21, 0.25, 0.21], C.rust)
    s.ellipsoid(
      [side * 1.0, -0.58, 0.47],
      [0.15, 0.19, 0.15],
      tint(C.teal, 0.88),
      0,
      290
    )
  }
  panel(s, [0, -0.3, 0.313], [0.4, 1.09, 0.035], C.ink)
  panel(s, [0, -0.31, 0.35], [0.36, 1.06, 0.06], C.gold, 2)
  panel(s, [0, -0.19, 0.389], [0.27, 0.68, 0.02], tint(C.blue, 0.65), 2)
  s.line([0.12, -0.28, 0.42], [0.12, -0.42, 0.42], 0.015, C.cream, 2, 12)
  panel(s, [0, -0.89, 0.44], [0.49, 0.085, 0.32], C.cream)
  awning(s, 0.7, 1.13, C.teal)
  clothWeights(s, 1.13)
  // The outline reads as a shopping bag even on the narrow canvas.
  panel(s, [0, 0.84, 0.02], [0.72, 0.39, 0.11], C.ink)
  const signZ = 0.085
  panel(s, [0, 0.8, signZ], [0.24, 0.17, 0.015], C.gold)
  s.tube(
    (t) => [
      Math.cos(t * Math.PI) * 0.07,
      0.88 + Math.sin(t * Math.PI) * 0.085,
      signZ,
    ],
    0.014,
    C.gold,
    0,
    22,
    6
  )
  for (const side of [-1, 1]) {
    s.line(
      [side * 0.17, 0.82, signZ],
      [side * 0.29, 0.82, signZ],
      0.01,
      C.cream,
      0,
      10
    )
  }
  return s.model(
    (t) => {
      const wind = breeze(t)
      const door = rotate(
        1,
        -0.52 * (0.5 - 0.5 * Math.cos(t * 0.82)) ** 2,
        [-0.18, 0, 0.35]
      )
      const lamp = Math.sin(t * 1.7) * 0.12
      return (p, v) => {
        if (p.closed) wind(p.closed, v)
        if (p.part === 2) door(p, v)
        if (p.part === 3) v[0] += (0.19 - p.y) * lamp
      }
    },
    [-0.29, -0.1]
  )
}
export function vending(): Sculpture {
  const s = new Shape()
  panel(s, [0, -0.02, 0], [1.4, 2.03, 0.8], C.rust)
  // A rolled rim separates the enamel body from the recessed glass display.
  const rim = tint(C.rust, 1.16)
  for (const x of [-0.67, 0.67]) {
    s.line([x, -0.93, 0.402], [x, 0.95, 0.402], 0.015, rim, 0, 89)
  }
  for (const y of [-0.94, 0.96]) {
    s.line([-0.67, y, 0.402], [0.67, y, 0.402], 0.015, rim, 0, 67)
  }
  panel(s, [-0.17, 0.24, 0.414], [0.92, 1.15, 0.045], C.ink)
  panel(s, [0, 0.89, 0.43], [1.14, 0.115, 0.04], C.cream)
  for (let row = 0; row < 3; row++) {
    const y = 0.6 - row * 0.32
    panel(s, [-0.17, y - 0.13, 0.48], [0.85, 0.03, 0.12], C.gold)
    for (let col = 0; col < 4; col++) {
      bottle(
        s,
        [-0.48 + col * 0.205, y, 0.51],
        [C.teal, C.gold, C.rose, C.blue][col],
        row === 2 && col === 2 ? 1 : 0
      )
    }
  }
  panel(s, [0.49, 0.49, 0.44], [0.24, 0.3, 0.03], C.ink)
  panel(s, [0.49, 0.55, 0.468], [0.16, 0.045, 0.015], C.teal)
  panel(s, [0.49, 0.38, 0.47], [0.15, 0.026, 0.015], C.cream)
  for (let i = 0; i < 4; i++) {
    s.ellipsoid(
      [0.49, 0.17 - i * 0.13, 0.47],
      [0.042, 0.042, 0.025],
      i === 2 ? C.gold : C.cream,
      0,
      80
    )
  }
  panel(s, [-0.14, -0.73, 0.43], [0.91, 0.3, 0.04], C.ink)
  panel(s, [-0.14, -0.89, 0.53], [0.96, 0.06, 0.26], C.gold)
  for (let i = 0; i < 4; i++) {
    s.line(
      [0.36, -0.65 - i * 0.055, 0.42],
      [0.61, -0.65 - i * 0.055, 0.42],
      0.009,
      C.ink,
      0,
      14
    )
  }
  for (const x of [-0.5, 0.5]) panel(s, [x, -1.08, 0], [0.17, 0.12, 0.6], C.ink)
  return s.model(
    (t) => {
      const phase = (t % 5.6) / 5.6
      const ramp = (start: number, end: number) =>
        smooth(Math.max(0, Math.min(1, (phase - start) / (end - start))))
      // Drop, tip into the tray, then restock behind the opaque cabinet.
      const drop = ramp(0.18, 0.38) ** 2 - ramp(0.73, 0.85)
      const tip = ramp(0.27, 0.43) - ramp(0.73, 0.85)
      const concealed = ramp(0.56, 0.72) - ramp(0.86, 0.98)
      const tilt = rotate(2, -tip * 1.45, [-0.07, -0.04, 0.51])
      return (p, v) => {
        if (p.part !== 1) return
        tilt(p, v)
        v[0] -= tip * 0.1
        v[1] -= drop * 0.74
        v[2] += drop * 0.08 - concealed * 0.34
      }
    },
    [-0.3, -0.08]
  )
}
export function stall(): Sculpture {
  const s = new Shape()
  awning(s, 0.98, 1.17, C.rust)
  clothWeights(s, 1.17)
  for (const x of [-0.93, 0.93]) {
    for (const z of [-0.45, 0.45]) {
      s.line([x, -0.87, z], [x, 0.86 - z * 0.16, z], 0.026, C.gold, 0, 38)
    }
  }
  panel(s, [0, -0.49, 0], [1.89, 0.61, 0.8], tint(C.teal, 0.71))
  panel(s, [0, -0.15, 0], [2.08, 0.092, 1.0], C.cream)
  for (const y of [-0.28, -0.68]) {
    panel(s, [0, y, 0.416], [1.87, 0.032, 0.025], C.gold)
  }
  for (const x of [-0.9, 0.9]) {
    panel(s, [x, -0.48, 0.423], [0.06, 0.52, 0.038], C.gold)
  }
  for (let tray = 0; tray < 3; tray++) {
    const x = (tray - 1) * 0.64,
      color = [C.gold, C.rose, C.teal][tray]
    panel(s, [x, -0.01, 0.08], [0.57, 0.14, 0.64], tint(C.rust, 0.75))
    for (let fruit = 0; fruit < 8; fruit++) {
      const upper = fruit >= 6
      const cx = x + (upper ? (fruit - 6.5) * 0.19 : ((fruit % 3) - 1) * 0.17)
      const cy = upper ? 0.23 : 0.12,
        cz = upper ? 0.06 : -0.045 + Math.floor(fruit / 3) * 0.2
      const height = [0.093, 0.085, 0.115][tray]
      s.ellipsoid(
        [cx, cy, cz],
        [0.093, height, 0.09],
        tint(color, 0.9 + (fruit % 3) * 0.05),
        0,
        145
      )
      if (upper) {
        s.line(
          [cx, cy + height * 0.85, cz],
          [cx + 0.024, cy + height + 0.035, cz],
          0.008,
          C.ink,
          0,
          5
        )
      }
    }
  }
  const pivot: Vec3 = [0.66, 0.74, 0.48]
  for (const x of [0.55, 0.77]) {
    s.line(pivot, [x, 0.53, 0.48], 0.01, C.gold, 2, 14)
  }
  panel(s, [0.66, 0.43, 0.49], [0.34, 0.22, 0.05], C.ink, 2)
  for (let i = 0; i < 2; i++) {
    s.line(
      [0.54, 0.47 - i * 0.08, 0.526],
      [0.76 - i * 0.07, 0.47 - i * 0.08, 0.526],
      0.01,
      C.cream,
      2,
      12
    )
  }
  for (const x of [-0.73, 0.73]) {
    const y = -0.94,
      z = 0.39,
      r = 0.17
    s.tube(
      (t) => [x + r * Math.cos(t * TAU), y + r * Math.sin(t * TAU), z],
      0.026,
      C.gold,
      0,
      65,
      8
    )
    for (let i = 0; i < 3; i++) {
      const dx = r * Math.cos((i * Math.PI) / 3),
        dy = r * Math.sin((i * Math.PI) / 3)
      s.line([x - dx, y - dy, z], [x + dx, y + dy, z], 0.012, C.cream, 0, 15)
    }
    s.ellipsoid([x, y, z], [0.032, 0.032, 0.035], C.rust, 0, 50)
  }
  return s.model(
    (t) => {
      const wind = breeze(t)
      const sign = rotate(2, Math.sin(t * 1.7) * 0.11, pivot)
      return (p, v) => {
        if (p.closed) wind(p.closed, v)
        if (p.part === 2) sign(p, v)
      }
    },
    [-0.31, -0.12]
  )
}
