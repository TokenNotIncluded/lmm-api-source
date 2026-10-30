/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, test } from 'node:test'

import { Window } from 'happy-dom'

const view = new Window({ url: 'https://example.test/' })
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'Node',
  'Element',
  'MutationObserver',
  'Event',
  'PointerEvent',
  'DOMRect',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: view[key],
  })
}
Object.defineProperty(view, 'devicePixelRatio', { value: 2 })
Object.defineProperty(view.document, 'hidden', {
  configurable: true,
  value: false,
})

let supported = false
let failSnapshot = false
let requests = 0
const draws: unknown[][] = []
const intersections = new Set<(visible: boolean) => void>()
let observers = 0

class Resize implements ResizeObserver {
  constructor(private callback: ResizeObserverCallback) {
    observers += 1
  }
  observe(target: Element) {
    this.callback(
      [
        {
          target,
          contentRect: new DOMRect(0, 0, 640, 480),
          borderBoxSize: [{ inlineSize: 640, blockSize: 480 }],
          contentBoxSize: [{ inlineSize: 640, blockSize: 480 }],
          devicePixelContentBoxSize: [{ inlineSize: 1280, blockSize: 960 }],
        },
      ],
      this
    )
  }
  disconnect() {
    observers -= 1
  }
  unobserve() {}
}
class Intersections {
  private update: (visible: boolean) => void
  constructor(callback: IntersectionObserverCallback) {
    observers += 1
    this.update = (isIntersecting) =>
      callback(
        [{ isIntersecting } as IntersectionObserverEntry],
        this as unknown as IntersectionObserver
      )
  }
  observe() {
    intersections.add(this.update)
    this.update(true)
  }
  disconnect() {
    intersections.delete(this.update)
    observers -= 1
  }
}
class Context {
  drawElementImage(...args: unknown[]) {
    if (failSnapshot) throw new Error('Snapshot is unavailable')
    draws.push(args)
  }
  clearRect() {}
  save() {}
  scale() {}
  restore() {}
  createRadialGradient() {
    return { addColorStop() {} }
  }
  fillRect() {}
}

Object.defineProperty(globalThis, 'ResizeObserver', {
  configurable: true,
  value: Resize,
})
Object.defineProperty(globalThis, 'IntersectionObserver', {
  configurable: true,
  value: Intersections,
})
Object.defineProperty(globalThis, 'CanvasRenderingContext2D', {
  configurable: true,
  value: Context,
})
Object.defineProperty(view.HTMLCanvasElement.prototype, 'content', {
  configurable: true,
  get() {
    return supported ? this.getAttribute('content') : undefined
  },
})
Object.defineProperty(view.HTMLCanvasElement.prototype, 'requestPaint', {
  configurable: true,
  value() {
    requests += 1
  },
})
Object.defineProperty(view.HTMLCanvasElement.prototype, 'getContext', {
  configurable: true,
  value() {
    return new Context()
  },
})

const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { HtmlCanvasMesh } = await import('./html-canvas-mesh')
;(
  globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

async function mount() {
  const host = document.createElement('main')
  host.className = 'lmm-home'
  document.body.append(host)
  const root = createRoot(host)
  await act(async () =>
    root.render(
      <HtmlCanvasMesh
        brandName='Tenant Studio'
        brandLogo='https://assets.example.test/logo.svg'
        t={(key) => key}
      />
    )
  )
  return {
    host,
    async cleanup() {
      await act(async () => root.unmount())
      host.remove()
    },
  }
}

after(() => view.close())

test('unsupported browsers keep one complete DOM scene and real links', async () => {
  supported = false
  const { host, cleanup } = await mount()
  assert.equal(host.querySelector('canvas'), null)
  assert.equal(host.querySelectorAll('.lmm-mesh-scene').length, 1)
  assert.equal(host.querySelectorAll('a[href="/pricing"]').length, 5)
  assert.equal(host.querySelectorAll('a[href="/guide"]').length, 2)
  assert.equal(
    host.querySelector('.lmm-mesh-core-name')?.textContent,
    'Tenant Studio'
  )
  assert.equal(
    host.querySelector('.lmm-mesh-brand')?.getAttribute('src'),
    'https://assets.example.test/logo.svg'
  )
  const stage = host.querySelector<HTMLElement>('.lmm-mesh-stage')
  assert.ok(stage)
  stage.dispatchEvent(
    new PointerEvent('pointermove', {
      clientX: 120,
      clientY: 90,
      pointerType: 'mouse',
    })
  )
  assert.equal(stage.hasAttribute('data-pointer'), true)
  const pause = host.querySelector<HTMLButtonElement>('button')
  assert.ok(pause)
  await act(async () => pause.click())
  assert.equal(pause.getAttribute('aria-pressed'), 'true')
  assert.equal(stage.hasAttribute('data-pointer'), false)
  await cleanup()
  assert.equal(observers, 0)
})

test('native paint uses device pixels, stops offscreen, and does not schedule an idle loop', async () => {
  supported = true
  failSnapshot = false
  draws.length = 0
  requests = 0
  const { host, cleanup } = await mount()
  const canvas = host.querySelector('canvas')
  assert.ok(canvas)
  assert.equal(canvas.getAttribute('content'), 'drawable')
  const requested = requests
  canvas.dispatchEvent(new Event('paint'))
  assert.equal(draws.length, 1)
  assert.equal((draws[0][0] as HTMLElement).hasAttribute('drawable'), true)
  assert.deepEqual(draws[0].slice(1), [0, 0, 1280, 960])
  assert.equal(
    requests,
    requested,
    'painting must not request another idle frame'
  )
  for (const update of intersections) update(false)
  canvas.dispatchEvent(new Event('paint'))
  assert.equal(draws.length, 1)
  for (const update of intersections) update(true)
  canvas.dispatchEvent(new Event('paint'))
  assert.equal(draws.length, 2)
  await cleanup()
  assert.equal(observers, 0)
})

test('a native snapshot failure restores the usable DOM scene', async () => {
  supported = true
  failSnapshot = true
  const { host, cleanup } = await mount()
  const canvas = host.querySelector('canvas')
  assert.ok(canvas)
  await act(async () => canvas.dispatchEvent(new Event('paint')))
  assert.equal(host.querySelector('canvas'), null)
  assert.equal(host.querySelectorAll('.lmm-mesh-scene').length, 1)
  assert.equal(host.querySelectorAll('a').length, 7)
  assert.equal(
    host.querySelector('.lmm-mesh-core-name')?.textContent,
    'Tenant Studio'
  )
  await cleanup()
  failSnapshot = false
  assert.equal(observers, 0)
})
