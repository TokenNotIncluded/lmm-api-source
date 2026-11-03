/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import { getSculpture, sequenceAt } from './home-sculptures'
import {
  hash,
  mix as lerp,
  smooth,
  type Point,
  type Vec3,
} from './sculptures/geometry'

type Pointer = {
  x: number
  y: number
  vx: number
  vy: number
  previousX: number
  previousY: number
  active: boolean
}
const pack = (r: number, g: number, b: number) =>
  ((255 << 24) |
    (Math.round(b) << 16) |
    (Math.round(g) << 8) |
    Math.round(r)) >>>
  0
/** Coverage and trails blend with the actual ground, never a black matte. */

export function posterPalette(background: Vec3) {
  const light = background[0] + background[1] + background[2] > 384
  const coverage = (pixel: number, alpha: number) =>
    pack(
      lerp(background[0], pixel & 255, alpha),
      lerp(background[1], (pixel >>> 8) & 255, alpha),
      lerp(background[2], (pixel >>> 16) & 255, alpha)
    )
  return {
    light,
    background: pack(...background),
    coverage,
    ink: (color: Vec3) => {
      // Preserve hue and shading, but give pale materials a printable midtone.
      const luminance =
        color[0] * 0.2126 + color[1] * 0.7152 + color[2] * 0.0722
      const tone = light ? 0.62 : 1
      const rgb = color.map((channel) => {
        const saturated = light
          ? luminance + (channel - luminance) * 1.2
          : channel
        return Math.min(255, Math.max(0, saturated * tone))
      }) as Vec3
      return coverage(pack(...rgb), light ? 1 : 0.96)
    },
  }
}

// Depth tones: far points recede into the ground, near points stay solid.
const TONES = 6
type Paint = {
  tones: Uint32Array
  trail: Uint32Array
}
const hexColor = (value: string, fallback: number) => {
  const hex = /^#([a-f\d]{6})$/i.exec(value.trim())
  return hex ? Number.parseInt(hex[1], 16) : fallback
}

export function createHomePoster(
  canvas: HTMLCanvasElement,
  onReady = () => {}
) {
  const context = canvas.getContext('2d', { alpha: false })
  if (!context) return null
  let alive = true,
    ready = false
  // Keep the fine-point lattice, sample budgets and brush springs.
  const count = canvas.clientWidth < 680 ? 22000 : 60000
  const offsets = new Float32Array(count * 4)
  const scatter = Float32Array.from(
    { length: count * 2 },
    (_, i) => hash(i) - 0.5
  )
  const ages = new Float64Array(5)
  const position: Vec3 = [0, 0, 0]
  let bitmap: ImageData | undefined,
    pixels: Uint32Array | undefined,
    depths: Float32Array | undefined,
    ground: Uint32Array | undefined
  let width = 0,
    height = 0,
    ratio = 1
  let colors = new WeakMap<Point[], Paint>()
  let palette = posterPalette([7, 7, 7])
  let themeKey: string | undefined
  const refreshTheme = () => {
    const key = document.documentElement.className
    if (key === themeKey) return
    themeKey = key
    const style = getComputedStyle(canvas)
    const color = hexColor(style.getPropertyValue('--poster-ground'), 0x070707)
    palette = posterPalette([
      (color >>> 16) & 255,
      (color >>> 8) & 255,
      color & 255,
    ])
    colors = new WeakMap()
    paintGround(hexColor(style.getPropertyValue('--poster-glow'), 0xd97757))
  }
  /** A soft halo and a faint print grid, settling to the exact ground at the edges. */
  const paintGround = (glow: number) => {
    if (!ground) return
    const tint = palette.coverage(
      pack((glow >>> 16) & 255, (glow >>> 8) & 255, glow & 255),
      1
    )
    const strength = palette.light ? 0.09 : 0.16
    const cx = canvas.width / 2,
      cy = canvas.height * 0.43,
      reach = Math.min(canvas.width, canvas.height) * 0.72
    const cell = Math.round(26 * ratio),
      grid = width >= 320
    const levels = Array.from({ length: 129 }, (_, i) =>
      palette.coverage(tint, strength * (i / 128) ** 3)
    )
    const dot = palette.coverage(
      palette.ink(palette.light ? [0, 0, 0] : [255, 255, 255]),
      palette.light ? 0.07 : 0.06
    )
    for (let row = 0; row < canvas.height; row++) {
      for (let col = 0; col < canvas.width; col++) {
        const d = Math.hypot(col - cx, row - cy) / reach
        const slot = row * canvas.width + col
        ground[slot] =
          grid && col % cell === cell >> 1 && row % cell === cell >> 1
            ? dot
            : levels[d < 1 ? Math.round((1 - d) * 128) : 0]
      }
    }
  }
  const depthAlpha = (tone: number) =>
    (palette.light ? 0.52 : 0.6) +
    ((palette.light ? 0.48 : 0.4) * tone) / (TONES - 1)
  const paintFor = (points: Point[]) => {
    let paint = colors.get(points)
    if (!paint) {
      paint = {
        tones: new Uint32Array(points.length * TONES),
        trail: new Uint32Array(points.length),
      }
      for (let i = 0; i < points.length; i++) {
        const pixel = palette.ink(points[i].color)
        for (let tone = 0; tone < TONES - 1; tone++)
          {paint.tones[i * TONES + tone] = palette.coverage(
            pixel,
            depthAlpha(tone)
          )}
        // The nearest points catch a little light, like a print highlight.
        const lift = palette.light ? 0 : 0.22
        paint.tones[i * TONES + TONES - 1] = pack(
          lerp(pixel & 255, 255, lift),
          lerp((pixel >>> 8) & 255, 240, lift),
          lerp((pixel >>> 16) & 255, 225, lift)
        )
        paint.trail[i] = palette.coverage(pixel, 0.14)
      }
      colors.set(points, paint)
    }
    return paint
  }
  return {
    draw(progress: number, pointer: Pointer, delta: number, still = false) {
      if (!alive) return
      const w = canvas.clientWidth,
        h = canvas.clientHeight
      if (!w || !h) return
      const nextRatio = Math.min(window.devicePixelRatio || 1, 1.5)
      if (width !== w || height !== h || ratio !== nextRatio) {
        ratio = nextRatio
        width = w
        height = h
        canvas.width = Math.round(w * ratio)
        canvas.height = Math.round(h * ratio)
        bitmap = context.createImageData(canvas.width, canvas.height)
        pixels = new Uint32Array(bitmap.data.buffer)
        depths = new Float32Array(canvas.width * canvas.height)
        ground = new Uint32Array(canvas.width * canvas.height)
        themeKey = undefined
      }
      refreshTheme()
      const chapter = Math.min(
        4,
        Math.max(0, Number.isFinite(progress) ? progress : 0)
      )
      const from = Math.floor(chapter),
        to = Math.min(4, from + 1),
        pageMix = smooth(chapter - from)
      const seconds = still
        ? 0
        : Math.min(0.08, Math.max(0, Number.isFinite(delta) ? delta : 0))
      if (!still) {
        ages[from] += seconds
        if (to !== from && pageMix > 0) ages[to] += seconds
      }
      const left = sequenceAt(from, ages[from]),
        right = sequenceAt(to, ages[to])
      canvas.dataset.sculpture = pageMix < 0.5 ? left.from : right.from
      // Free-floating stipple: no lattice, so no brick rows or moiré bands.
      const size = Math.max(1, Math.round((w < 680 ? 1.5 : 2.2) * ratio)),
        round = size >= 3,
        trailStart = size + 1
      const inputs = [
        {
          id: left.from,
          weight: (1 - pageMix) * (1 - left.mix),
          age: left.age,
        },
        { id: left.to, weight: (1 - pageMix) * left.mix, age: left.nextAge },
        { id: right.from, weight: pageMix * (1 - right.mix), age: right.age },
        { id: right.to, weight: pageMix * right.mix, age: right.nextAge },
      ]
      const layers = inputs
        .filter((l) => l.weight > 0)
        .map((layer) => {
          const model = getSculpture(layer.id),
            [yaw, pitch] = model.view ?? [0, 0]
          return {
            ...layer,
            model,
            pose: model.animate(layer.age),
            paint: paintFor(model.points),
            cy: Math.cos(yaw),
            sy: Math.sin(yaw),
            cp: Math.cos(pitch),
            sp: Math.sin(pitch),
          }
        })
      const spread =
        (1 - pageMix) * Math.sin(Math.PI * left.mix) ** 2 +
        pageMix * Math.sin(Math.PI * right.mix) ** 2
      const n = Math.min(
        count,
        Math.max(...layers.map((l) => l.model.points.length))
      )
      if (!n || !bitmap || !pixels || !depths || !ground) return
      pixels.set(ground)
      depths.fill(-Infinity)
      // Leave room for dispersal in every direction, then return to the resting frame.
      const scale = Math.min(w * 0.34, h * 0.35) * (1 - spread * 0.3),
        centerX = w / 2,
        centerY = h * (0.43 + spread * 0.07)
      const stride = canvas.width,
        rows = canvas.height
      const brushX = pointer.x - pointer.previousX,
        brushY = pointer.y - pointer.previousY
      const segment = brushX * brushX + brushY * brushY
      const speed = Math.min(1600, Math.hypot(pointer.vx, pointer.vy))
      const radius = Math.min(w, h) * (0.22 + speed / 22000)
      const recovery = Math.exp(-20 * seconds)
      for (let i = 0; i < n; i++) {
        let x = 0,
          y = 0,
          z = 0,
          red = 0,
          green = 0,
          blue = 0
        let tone = 0,
          pixel = 0,
          trail = 0
        for (const layer of layers) {
          const sample = Math.floor((i / n) * layer.model.points.length),
            p = layer.model.points[sample]
          position[0] = p.x
          position[1] = p.y
          position[2] = p.z
          layer.pose(p, position)
          const rx = position[0] * layer.cy + position[2] * layer.sy
          const rz = position[2] * layer.cy - position[0] * layer.sy
          x += rx * layer.weight
          y += (position[1] * layer.cp + rz * layer.sp) * layer.weight
          z += (rz * layer.cp - position[1] * layer.sp) * layer.weight
          tone = sample * TONES + TONES - 1
          pixel = layer.paint.tones[tone]
          if (layers.length === 1) {
            trail = layer.paint.trail[sample]
          } else {
            red += (pixel & 255) * layer.weight
            green += ((pixel >>> 8) & 255) * layer.weight
            blue += ((pixel >>> 16) & 255) * layer.weight
          }
        }
        x += scatter[i * 2] * spread * 1.8
        y += (scatter[i * 2 + 1] * 1.25 + 0.16) * spread
        z += scatter[i * 2] * spread * 0.45
        const depth = Math.min(
          TONES - 1,
          Math.max(0, Math.round((z + 0.9) * 0.55 * (TONES - 1)))
        )
        if (layers.length > 1) {
          const solid = pack(red, green, blue)
          pixel = palette.coverage(solid, depthAlpha(depth))
          trail = palette.coverage(solid, 0.14)
        } else {
          pixel = layers[0].paint.tones[tone - (TONES - 1) + depth]
        }
        const perspective = 3.9 / (3.9 - z * 0.42)
        const px = centerX + x * scale * perspective
        const py = centerY - y * scale * perspective
        const index = i * 4
        let fx = 0,
          fy = 0
        if (pointer.active && !still) {
          const along = segment
            ? Math.max(
                0,
                Math.min(
                  1,
                  ((px - pointer.previousX) * brushX +
                    (py - pointer.previousY) * brushY) /
                    segment
                )
              )
            : 1
          const dx = px - (pointer.previousX + brushX * along),
            dy = py - (pointer.previousY + brushY * along)
          const distance = Math.hypot(dx, dy)
          if (distance < radius) {
            const force = Math.pow(1 - distance / radius, 2)
            fx =
              force *
              (pointer.vx * 0.16 +
                dx * 0.25 +
                scatter[i * 2] * (35 + speed * 0.06))
            fy =
              force *
              (pointer.vy * 0.07 +
                dy * 0.35 +
                scatter[i * 2 + 1] * (30 + speed * 0.09))
          }
        }
        // Closed-form critical damping: return time does not depend on fps.
        const gapX = offsets[index] - fx,
          gapY = offsets[index + 1] - fy
        const springX = offsets[index + 2] + 20 * gapX,
          springY = offsets[index + 3] + 20 * gapY
        offsets[index] = fx + (gapX + springX * seconds) * recovery
        offsets[index + 1] = fy + (gapY + springY * seconds) * recovery
        offsets[index + 2] =
          (offsets[index + 2] - 20 * springX * seconds) * recovery
        offsets[index + 3] =
          (offsets[index + 3] - 20 * springY * seconds) * recovery
        const ox = offsets[index],
          oy = offsets[index + 1]
        const dx = Math.round((px + ox) * ratio),
          dy = Math.round((py + oy) * ratio)
        for (
          let row = Math.max(0, dy);
          row < Math.min(rows, dy + size);
          row++
        ) {
          const edgeRow = row === dy || row === dy + size - 1
          for (
            let col = Math.max(0, dx);
            col < Math.min(stride, dx + size);
            col++
          ) {
            // Rounded dots: drop the four corners of larger brushes.
            if (round && edgeRow && (col === dx || col === dx + size - 1))
              {continue}
            const slot = row * stride + col
            if (z < depths[slot]) continue
            depths[slot] = z
            pixels[slot] = pixel
          }
        }
        if (Math.abs(ox) > 2) {
          const ty = Math.round((py + oy * 0.75) * ratio),
            tx = Math.round((px + ox * 0.62) * ratio)
          const length = Math.round(
            Math.min(55, Math.abs(ox) * 0.36) * ratio + trailStart
          )
          if (ty >= 0 && ty < rows) {
            for (
              let col = Math.max(0, tx);
              col < Math.min(stride, tx + length);
              col++
            ) {
              const slot = ty * stride + col
              if (depths[slot] === -Infinity) pixels[slot] = trail
            }
          }
        }
      }
      context.putImageData(bitmap, 0, 0)
      // Notify only after a real frame. A microtask avoids re-entering the owner.
      if (!ready) {
        ready = true
        canvas.dataset.ready = 'true'
        queueMicrotask(() => {
          if (alive) onReady()
        })
      }
    },
    dispose() {
      alive = false
      delete canvas.dataset.ready
      delete canvas.dataset.sculpture
      canvas.width = 0
      canvas.height = 0
      bitmap = undefined
      pixels = undefined
      depths = undefined
      ground = undefined
      colors = new WeakMap()
    },
  }
}
