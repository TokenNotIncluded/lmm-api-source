/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import assert from 'node:assert/strict'
import { after, test } from 'node:test'

import { Window } from 'happy-dom'

import type { RemoteCommandInput } from './commands'
import type { RemoteQuestion } from './types'

const dom = new Window()
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'Node',
  'Element',
  'Event',
  'MouseEvent',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: dom[key],
  })
}
Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
  configurable: true,
  value: true,
})
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const i18next = (await import('i18next')).default
const { initReactI18next } = await import('react-i18next')
await i18next.use(initReactI18next).init({
  lng: 'en',
  resources: { en: { translation: {} } },
})
const { RemoteQuestionCard } = await import('./controls')
after(() => dom.close())

async function mountQuestion(question: RemoteQuestion, disabled = false) {
  let now = 1_900_000_000_000
  let nextTimer = 0
  const timers = new Map<number, { due: number; run: () => void }>()
  const realNow = Date.now
  const realTimeout = window.setTimeout
  const realClear = window.clearTimeout
  Date.now = () => now
  window.setTimeout = ((handler: TimerHandler, delay = 0) => {
    assert.equal(typeof handler, 'function')
    const id = ++nextTimer
    timers.set(id, { due: now + delay, run: handler as () => void })
    return id
  }) as typeof window.setTimeout
  window.clearTimeout = ((id: number) => {
    timers.delete(id)
  }) as typeof window.clearTimeout
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  let mounted = true
  const sent: RemoteCommandInput[] = []
  const send = async (command: RemoteCommandInput) => {
    sent.push(command)
  }
  const render = async (next: RemoteQuestion, blocked = false) => {
    await act(async () =>
      root.render(
        <RemoteQuestionCard question={next} disabled={blocked} send={send} />
      )
    )
  }
  const unmount = async () => {
    if (!mounted) return
    await act(async () => root.unmount())
    mounted = false
    host.remove()
  }
  const restore = () => {
    Date.now = realNow
    window.setTimeout = realTimeout
    window.clearTimeout = realClear
  }
  try {
    await render(question, disabled)
  } catch (error) {
    await unmount()
    restore()
    throw error
  }
  return {
    host,
    sent,
    timers,
    render,
    unmount,
    restore,
    async advance(ms: number, fire = true) {
      now += ms
      if (!fire) return
      await act(async () => {
        for (const [id, timer] of timers) {
          if (timer.due <= now) {
            timers.delete(id)
            timer.run()
          }
        }
      })
    },
  }
}

const start = 1_900_000_000_000
function question(
  kind: RemoteQuestion['kind'],
  expires_at?: number
): RemoteQuestion {
  return {
    request_id: 'request_fixture',
    kind,
    question: 'Fixture question',
    options: ['First', 'Second'],
    expires_at,
  }
}

for (const kind of ['input', 'confirm', 'select', 'custom'] as const) {
  test(`${kind} expires in milliseconds without a parent render and cannot send`, async () => {
    const view = await mountQuestion(question(kind, start + 50))
    try {
      const controls = () => [
        ...view.host.querySelectorAll<
          HTMLButtonElement | HTMLInputElement | HTMLTextAreaElement
        >('button, input, textarea'),
      ]
      assert.ok(controls().some((control) => !control.disabled))
      await view.advance(49)
      assert.ok(controls().some((control) => !control.disabled))
      await view.advance(1)
      assert.ok(controls().every((control) => control.disabled))
      await act(async () => {
        for (const button of view.host.querySelectorAll('button')) {
          button.click()
        }
        view.host
          .querySelector('form')
          ?.dispatchEvent(
            new Event('submit', { bubbles: true, cancelable: true })
          )
      })
      assert.deepEqual(view.sent, [])
    } finally {
      await view.unmount()
      view.restore()
    }
  })

  test(`${kind} rechecks the deadline when its timer has not run`, async () => {
    const view = await mountQuestion(question(kind, start + 50))
    try {
      await view.advance(50, false)
      await act(async () => {
        const form = view.host.querySelector('form')
        if (form) {
          form.dispatchEvent(
            new Event('submit', { bubbles: true, cancelable: true })
          )
        } else {
          const button = view.host.querySelector('button')
          assert.ok(button)
          assert.equal(button.disabled, false)
          button.click()
        }
      })
      assert.deepEqual(view.sent, [])
    } finally {
      await view.unmount()
      view.restore()
    }
  })
}

test('question deadline updates cancel old timers, re-enable renewed questions and clean up on unmount', async () => {
  const view = await mountQuestion(question('confirm', start + 50))
  try {
    const oldTimer = [...view.timers.keys()][0]
    assert.ok(oldTimer)
    await view.render({
      ...question('confirm', start + 100),
      request_id: 'request_updated',
    })
    assert.equal(view.timers.has(oldTimer), false)
    assert.equal(view.timers.size, 1)
    await view.advance(50)
    assert.equal(view.host.querySelector('button')?.disabled, false)
    await view.advance(50)
    assert.equal(view.host.querySelector('button')?.disabled, true)
    await view.render(question('confirm', start + 200))
    assert.equal(view.host.querySelector('button')?.disabled, false)
    assert.equal(view.timers.size, 1)
    await view.render(question('confirm'))
    assert.equal(view.timers.size, 0)
    await view.render(question('confirm', start + 200))
    await view.unmount()
    assert.equal(view.timers.size, 0)
  } finally {
    await view.unmount()
    view.restore()
  }
})

test('unexpired questions preserve permissions and the response payload; already expired questions are blocked', async () => {
  const current = question('confirm', start + 50)
  const view = await mountQuestion(current, true)
  try {
    assert.equal(view.host.querySelector('button')?.disabled, true)
    await view.render(current)
    const confirm = view.host.querySelector('button')
    assert.ok(confirm)
    await act(async () => confirm.click())
    assert.deepEqual(view.sent, [
      { action: 'ui_response', request_id: current.request_id, value: true },
    ])
    await view.render(question('confirm', start))
    assert.equal(view.host.querySelector('button')?.disabled, true)
  } finally {
    await view.unmount()
    view.restore()
  }
})
