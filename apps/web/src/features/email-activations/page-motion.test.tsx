/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.

For commercial licensing, please contact support@quantumnous.com
*/
/*
Copyright (C) 2026 LIghtJUNction
*/
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { after, describe, test } from 'node:test'

import { Window } from 'happy-dom'
import type { ReactNode } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'

import {
  CARD_ITEM_VARIANTS,
  CARD_STAGGER_VARIANTS,
  MOTION_TRANSITION,
  MOTION_VARIANTS,
} from '@/lib/motion'

const identityT = (value: string) => value

const domWindow = new Window()
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'SVGElement',
  'Node',
  'Element',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: domWindow[key],
  })
}

for (const timer of [
  'requestAnimationFrame',
  'cancelAnimationFrame',
] as const) {
  Object.defineProperty(globalThis, timer, {
    configurable: true,
    value: domWindow[timer].bind(domWindow),
  })
}

// Load browser-dependent modules after installing the test window.
const { flushSync } = await import('react-dom')
const { createRoot } = await import('react-dom/client')
const { CardStaggerContainer, CardStaggerItem, FadeIn } =
  await import('@/components/page-transition')
const { Button, buttonVariants } = await import('@/components/ui/button')
const { HeroSmsStatusBadge } = await import('./status')

function setMotionPreference(reduce: boolean) {
  domWindow.happyDOM.settings.device.prefersReducedMotion = reduce
    ? 'reduce'
    : 'no-preference'
  domWindow.dispatchEvent(new domWindow.Event('resize'))
}

async function renderWithMotion(reduce: boolean, children: ReactNode) {
  setMotionPreference(reduce)
  const container = document.createElement('div')
  const root = createRoot(container)
  try {
    flushSync(() => root.render(children))
    // happy-dom evaluates media-query change events on resize.
    domWindow.dispatchEvent(new domWindow.Event('resize'))
    const initialMarkup = container.innerHTML
    const animations = [...container.querySelectorAll('*')].flatMap((node) =>
      node.getAnimations()
    )
    await Promise.all(animations.map((animation) => animation.finished))
    return initialMarkup
  } finally {
    flushSync(() => root.unmount())
  }
}

describe('email activation page motion', () => {
  after(() => domWindow.close())

  test('server-rendered feedback stays visible before a motion preference is known', () => {
    const markup = renderToStaticMarkup(
      <FadeIn className='feedback-wrap'>
        <span>Purchase failed</span>
      </FadeIn>
    )
    assert.match(markup, /^<div class="feedback-wrap">/)
    assert.doesNotMatch(markup, /opacity/)
    assert.match(markup, /Purchase failed/)
  })

  test('FadeIn renders the final state directly under reduced motion', async () => {
    const markup = await renderWithMotion(
      true,
      <FadeIn className='feedback-wrap'>
        <span>Purchase failed</span>
      </FadeIn>
    )
    assert.match(markup, /^<div class="feedback-wrap">/)
    assert.doesNotMatch(markup, /opacity/)
    assert.match(markup, /Purchase failed/)
  })

  test('CardStaggerContainer renders a plain container under reduced motion', async () => {
    const markup = await renderWithMotion(
      true,
      <CardStaggerContainer className='history-list'>
        <span>item</span>
      </CardStaggerContainer>
    )
    assert.match(markup, /^<div class="history-list">/)
    assert.doesNotMatch(markup, /opacity/)
  })

  test('FadeIn animates opacity 0 -> 1 when motion is allowed', async () => {
    const markup = await renderWithMotion(
      false,
      <FadeIn className='feedback-wrap'>
        <span>Purchase reconciling</span>
      </FadeIn>
    )
    // The initial variant is rendered inline; the shared transition budget
    // used by this page stays inside 150–250ms.
    assert.match(markup, /opacity:\s*0/)
    assert.ok((MOTION_TRANSITION.default.duration ?? 0) <= 0.25)
    assert.ok((MOTION_TRANSITION.default.duration ?? 0) >= 0.15)
    assert.ok((MOTION_TRANSITION.fast.duration ?? 0) <= 0.25)
  })

  test('changing to reduced motion reveals a delayed feedback message without remounting it', async () => {
    setMotionPreference(false)
    const container = document.body.appendChild(document.createElement('div'))
    const root = createRoot(container)
    try {
      flushSync(() =>
        root.render(
          <FadeIn delay={2}>
            <input />
          </FadeIn>
        )
      )
      domWindow.dispatchEvent(new domWindow.Event('resize'))
      const wrapper = container.querySelector('div')
      const input = container.querySelector('input')
      assert.ok(wrapper && input)
      assert.equal(wrapper.style.opacity, '0')
      input.focus()
      input.value = 'draft'
      const pending = wrapper
        .getAnimations()
        .map((animation) =>
          animation.finished.catch((error: Error) =>
            assert.equal(error.name, 'AbortError')
          )
        )
      flushSync(() => setMotionPreference(true))
      await new Promise<void>((resolve) =>
        requestAnimationFrame(() => resolve())
      )
      assert.equal(wrapper.style.opacity, '1')
      assert.equal(wrapper.style.transition, 'none')
      assert.ok(document.activeElement === input)
      assert.equal(container.querySelector('input')?.value, 'draft')
      await Promise.all(pending)
    } finally {
      flushSync(() => root.unmount())
      container.remove()
    }
  })

  test('stagger wrappers render list content with compliant variants', async () => {
    const markup = await renderWithMotion(
      false,
      <CardStaggerContainer className='history-list'>
        <CardStaggerItem>
          <span>No email activations match the current filter.</span>
        </CardStaggerItem>
      </CardStaggerContainer>
    )
    // Variants may start hidden but must settle to opacity 1 / identity
    // transform within the shared budget.
    assert.deepEqual(CARD_STAGGER_VARIANTS.initial, {})
    assert.deepEqual(CARD_ITEM_VARIANTS.animate, {
      opacity: 1,
      y: 0,
      scale: 1,
      transition: MOTION_TRANSITION.default,
    })
    const staggerChildren = (
      CARD_STAGGER_VARIANTS.animate as {
        transition?: { staggerChildren?: number }
      }
    ).transition?.staggerChildren
    assert.ok((staggerChildren ?? 1) <= 0.05)
    assert.match(markup, /opacity:\s*0/)
    assert.match(markup, /No email activations match the current filter\./)
  })

  test('page-used variants and transitions stay inside the opacity/transform 250ms budget', () => {
    // Only the transitions this page consumes are bound here; slow/spring/none
    // are shared library constants used by other surfaces (baseline).
    assert.ok((MOTION_TRANSITION.default.duration ?? 0) <= 0.25)
    assert.ok((MOTION_TRANSITION.fast.duration ?? 0) <= 0.25)
    assert.deepEqual(MOTION_VARIANTS.fadeIn.initial, { opacity: 0 })
    assert.deepEqual(MOTION_VARIANTS.fadeIn.animate, { opacity: 1 })

    const allowedKeys = new Set(['opacity', 'y', 'x', 'scale', 'filter'])
    for (const variant of [MOTION_VARIANTS.fadeIn, MOTION_VARIANTS.slideDown]) {
      const propertyKeys = new Set([
        ...Object.keys(variant.initial),
        ...Object.keys(variant.animate),
      ])
      for (const key of propertyKeys) {
        assert.ok(
          allowedKeys.has(key),
          `unexpected animated property "${key}" used by the page`
        )
      }
    }
  })

  test('status badge pairs an aria-label with visible text, not color alone', () => {
    const markup = renderToStaticMarkup(
      <HeroSmsStatusBadge status='active' t={identityT as never} />
    )
    assert.match(markup, /aria-label="Awaiting code"/)
    assert.match(markup, />Awaiting code</)
    assert.match(markup, /aria-hidden="true"/)
  })

  test('icon-only controls in the page and shared primitives expose accessible names', () => {
    // The page's TabsList is a pure icon-only control; it must carry an
    // aria-label through the shared `t()` pattern.
    const pageSource = readFileSync(
      join(import.meta.dirname, 'page.tsx'),
      'utf8'
    )
    assert.match(pageSource, /<TabsList[^>]*aria-label=\{t\(/)
    // CopyButton is icon-only by default and owns a default accessible name.
    const copyButtonSource = readFileSync(
      join(import.meta.dirname, '..', '..', 'components', 'copy-button.tsx'),
      'utf8'
    )
    assert.match(
      copyButtonSource,
      /aria-label=\{isCopied \? copiedAriaLabel : resolvedAriaLabel\}/
    )
    // The status badge component wires the same pattern.
    const statusSource = readFileSync(
      join(import.meta.dirname, 'status.tsx'),
      'utf8'
    )
    assert.match(statusSource, /aria-label=\{presentation\.label\}/)
  })

  test('primary buttons keep a visible focus ring for keyboard users', () => {
    assert.match(buttonVariants(), /focus-visible:/)
    const markup = renderToStaticMarkup(
      <Button onClick={() => {}}>{identityT('Refresh')}</Button>
    )
    assert.match(markup, /focus-visible:/)
    assert.match(markup, /<button/)
  })
})
