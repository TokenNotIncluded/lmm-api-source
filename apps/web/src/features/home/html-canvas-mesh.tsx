/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import { useEffect, useRef, useState } from 'react'

import { BrandLogo } from '@/components/brand-logo'

import './html-canvas-mesh.css'

type DrawableCanvas = HTMLCanvasElement & {
  content: string
  requestPaint: () => void
}

type DrawableContext = CanvasRenderingContext2D & {
  drawElementImage: (
    element: HTMLElement,
    x: number,
    y: number,
    width: number,
    height: number
  ) => void
}

type MeshProps = {
  brandName: string
  brandLogo: string
  pricingHref: string | null
  guideHref: string | null
  t: (key: string) => string
}

const NODES = [
  { label: 'Chat', path: 'M5 5h14v10H9l-4 4V5Z', x: 15, y: 29 },
  {
    label: 'Drawing',
    path: 'M4 4h16v16H4zM4 16l6-6 10 10M15 8h.01',
    x: 15,
    y: 71,
  },
  {
    label: 'Audio',
    path: 'M4 10v4m4-8v12m4-15v18m4-15v12m4-8v4',
    x: 85,
    y: 29,
  },
  { label: 'Code', path: 'm8 6-6 6 6 6m8-12 6 6-6 6m-3-14-2 16', x: 85, y: 71 },
  {
    label: 'Reasoning',
    path: 'M12 3v6m0 0-6 5m6-5 6 5M6 14v7m12-7v7M3 14h6m6 0h6',
    x: 50,
    y: 12,
  },
  {
    label: 'Vision',
    path: 'M2 12s4-7 10-7 10 7 10 7-4 7-10 7S2 12 2 12Zm10-3a3 3 0 1 0 0 6 3 3 0 0 0 0-6Z',
    x: 50,
    y: 88,
  },
] as const

function supportsHtmlCanvas() {
  if (typeof document === 'undefined') return false
  const canvas = document.createElement('canvas') as DrawableCanvas
  canvas.setAttribute('content', 'drawable')
  return (
    canvas.content === 'drawable' &&
    typeof canvas.requestPaint === 'function' &&
    typeof CanvasRenderingContext2D !== 'undefined' &&
    'drawElementImage' in CanvasRenderingContext2D.prototype
  )
}

/** One React scene, rendered either as live drawable HTML or regular HTML. */
export function HtmlCanvasMesh({
  brandName,
  brandLogo,
  pricingHref,
  guideHref,
  t,
}: MeshProps) {
  const CoreNode = guideHref ? 'a' : 'span'
  const stageRef = useRef<HTMLDivElement>(null)
  const sceneRef = useRef<HTMLDivElement>(null)
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const [native, setNative] = useState(supportsHtmlCanvas)
  const [paused, setPaused] = useState(false)

  useEffect(() => {
    const stage = stageRef.current
    const scene = sceneRef.current
    if (!stage || !scene) return
    const canvas = canvasRef.current as DrawableCanvas | null
    const ctx = canvas?.getContext('2d') as DrawableContext | null
    if (native && !ctx) {
      setNative(false)
      return
    }

    const reduced = window.matchMedia('(prefers-reduced-motion: reduce)')
    const home = stage.closest<HTMLElement>('.lmm-home')
    let visible = false
    let width = 0
    let height = 0
    const canMove = () =>
      !paused &&
      !reduced.matches &&
      !document.hidden &&
      visible &&
      home?.dataset.motion !== 'paused'

    const requestPaint = () => {
      if (canvas && visible && !document.hidden) {
        canvas.requestPaint()
      }
    }
    const resetPointer = () => {
      stage.removeAttribute('data-pointer')
      requestPaint()
    }
    const updateMotion = () => {
      stage.dataset.paused = String(!canMove())
      if (!canMove()) resetPointer()
      else requestPaint()
    }
    const onPointer = (event: PointerEvent) => {
      if (!canMove() || event.pointerType === 'touch') return
      stage.dataset.pointer = ''
      requestPaint()
    }
    const onPaint = () => {
      if (!canvas || !ctx || !visible || document.hidden || !width || !height) {
        return
      }
      try {
        ctx.clearRect(0, 0, canvas.width, canvas.height)
        // Explicit destination size avoids mixing CSS pixels with canvas pixels.
        // The browser updates the live HTML hit targets and accessibility bounds.
        ctx.drawElementImage(scene, 0, 0, canvas.width, canvas.height)
      } catch {
        // A failed DOM snapshot must leave the real links and text usable.
        setNative(false)
      }
    }
    const onContextLost = () => setNative(false)
    const resize = new ResizeObserver(([entry]) => {
      width = entry.contentRect.width
      height = entry.contentRect.height
      if (!canvas || !width || !height) return
      scene.style.width = `${width}px`
      scene.style.height = `${height}px`
      const pixels = entry.devicePixelContentBoxSize?.[0]
      const nextWidth =
        pixels?.inlineSize ?? Math.round(width * window.devicePixelRatio)
      const nextHeight =
        pixels?.blockSize ?? Math.round(height * window.devicePixelRatio)
      if (canvas.width !== nextWidth) canvas.width = nextWidth
      if (canvas.height !== nextHeight) canvas.height = nextHeight
      requestPaint()
    })
    const visibility = new IntersectionObserver(([entry]) => {
      visible = entry.isIntersecting
      updateMotion()
    })
    const motion = new MutationObserver(updateMotion)
    if (home) {
      motion.observe(home, {
        attributes: true,
        attributeFilter: ['data-motion'],
      })
    }
    const devicePixels =
      typeof ResizeObserverEntry !== 'undefined' &&
      'devicePixelContentBoxSize' in ResizeObserverEntry.prototype
    resize.observe(
      stage,
      devicePixels ? { box: 'device-pixel-content-box' } : {}
    )
    visibility.observe(stage)
    stage.addEventListener('pointermove', onPointer, { passive: true })
    stage.addEventListener('pointerleave', resetPointer)
    document.addEventListener('visibilitychange', updateMotion)
    reduced.addEventListener('change', updateMotion)
    canvas?.addEventListener('paint', onPaint)
    canvas?.addEventListener('contextlost', onContextLost)
    updateMotion()

    return () => {
      resize.disconnect()
      visibility.disconnect()
      motion.disconnect()
      stage.removeEventListener('pointermove', onPointer)
      stage.removeEventListener('pointerleave', resetPointer)
      document.removeEventListener('visibilitychange', updateMotion)
      reduced.removeEventListener('change', updateMotion)
      canvas?.removeEventListener('paint', onPaint)
      canvas?.removeEventListener('contextlost', onContextLost)
    }
  }, [native, paused])

  const scene = (
    <div
      className='lmm-mesh-scene'
      ref={sceneRef}
      {...(native ? { drawable: '' } : {})}
    >
      <svg
        className='lmm-mesh-wires'
        viewBox='0 0 1000 560'
        preserveAspectRatio='none'
        aria-hidden='true'
      >
        {NODES.map(({ label, x, y }) => {
          const wire = `M500 280 L${x * 10} ${y * 5.6}`
          return (
            <g key={label}>
              <path d={wire} />
              <path className='lmm-mesh-signal' d={wire} pathLength='100' />
            </g>
          )
        })}
        <circle cx='500' cy='280' r='4' />
      </svg>
      {NODES.map(({ label, path, x, y }) => {
        const href = label === 'Code' ? guideHref : pricingHref
        const Node = href ? 'a' : 'span'
        return (
          <Node
            key={label}
            className='lmm-mesh-node'
            href={href ?? undefined}
            style={{ left: `${x}%`, top: `${y}%` }}
          >
            <svg
              viewBox='0 0 24 24'
              fill='none'
              stroke='currentColor'
              strokeWidth='1.35'
              strokeLinecap='round'
              strokeLinejoin='round'
              aria-hidden='true'
            >
              <path d={path} />
            </svg>
            <span>{t(label)}</span>
          </Node>
        )
      })}
      <CoreNode
        className='lmm-mesh-core'
        href={guideHref ?? undefined}
        aria-label={brandName}
      >
        <span className='lmm-mesh-core-caption'>{t('One endpoint')}</span>
      </CoreNode>
    </div>
  )

  return (
    <figure className='lmm-mesh-figure'>
      <div
        className='lmm-mesh-stage'
        ref={stageRef}
        data-renderer={native ? 'html-in-canvas' : 'dom'}
      >
        {native ? (
          <canvas ref={canvasRef} content='drawable'>
            {scene}
          </canvas>
        ) : (
          scene
        )}
        {/* Custom tenant logos remain ordinary images, including cross-origin ones. */}
        <span className='lmm-mesh-brand' aria-hidden='true'>
          <BrandLogo src={brandLogo} className='lmm-brand-icon' />
        </span>
      </div>
      <figcaption className='lmm-mesh-caption'>
        <span>{t('Multimodal request routing')}</span>
        <button
          type='button'
          onClick={() => setPaused(!paused)}
          aria-pressed={paused}
        >
          <svg
            viewBox='0 0 16 16'
            fill='none'
            stroke='currentColor'
            strokeWidth='1.3'
            aria-hidden='true'
          >
            {paused ? <path d='m5 3 7 5-7 5Z' /> : <path d='M5 3v10M11 3v10' />}
          </svg>
          {t(paused ? 'Resume' : 'Pause')}
        </button>
      </figcaption>
    </figure>
  )
}
