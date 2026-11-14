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
import assert from 'node:assert/strict'
import { after, afterEach, beforeEach, describe, test } from 'node:test'

import { Window } from 'happy-dom'

const domWindow = new Window()
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'SVGElement',
  'Node',
  'Element',
  'Event',
  'MouseEvent',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: domWindow[key],
  })
}

const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { SignalGamePanel } = await import('./panel')
const { startSignalGame, useSignalGame } = await import('./store')
const { useAuthStore } = await import('@/stores/auth-store')
const { RULES_VERSION } = await import('./engine')

const originalGet = api.get
const originalPost = api.post
const previousAuth = useAuthStore.getState().auth

const i18n = createInstance()
await i18n.use(initReactI18next).init({
  lng: 'en',
  resources: { en: { translation: {} } },
})

const reactTestGlobals = globalThis as typeof globalThis & {
  IS_REACT_ACT_ENVIRONMENT?: boolean
}
reactTestGlobals.IS_REACT_ACT_ENVIRONMENT = true

async function renderArtwork() {
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  await act(async () =>
    root.render(
      <I18nextProvider i18n={i18n}>
        <SignalGamePanel />
      </I18nextProvider>
    )
  )
  return { container, root }
}

beforeEach(async () => {
  await startSignalGame('practice', 5)
  useSignalGame.setState({
    storageError: false,
    serviceError: false,
    records: [],
    rounds: 0,
  })
})

afterEach(() => {
  api.post = originalPost
  useAuthStore.setState({ auth: previousAuth })
  api.get = originalGet
  document.body.replaceChildren()
})
after(() => domWindow.close())

function control(container: Element, label: string) {
  const found = [
    ...container.querySelectorAll<HTMLButtonElement>('button'),
  ].find((item) => item.textContent?.trim() === label)
  assert.ok(found, `Missing game action ${label}`)
  return found
}

function login(sid: string) {
  useAuthStore.getState().auth.setBundle({
    access_token: 'synthetic-signal-session',
    token_type: 'Bearer',
    access_expires_at: 2_000_000_000,
    user: { id: 501, role: 1, username: 'signal-player' },
    session: {
      sid,
      current: true,
      login_method: 'password',
      ip: '127.0.0.1',
      user_agent: 'synthetic',
      created_at: 1,
      last_active_at: 1,
      expires_at: 2_000_000_000,
    },
  })
}

describe('SignalGamePanel optional game', () => {
  test('plays, hints, wins and restarts without network requests', async () => {
    let requests = 0
    api.get = (async () => {
      requests++
      throw new Error('game must stay local')
    }) as typeof api.get
    const rendered = await renderArtwork()
    try {
      const tiles = [
        ...rendered.container.querySelectorAll<HTMLButtonElement>(
          'button[aria-label^="Rotate tile"]'
        ),
      ]
      assert.equal(tiles.length, 25)
      assert.ok(tiles.every((button) => button.type === 'button'))
      const tile = tiles[10]
      const label = tile.getAttribute('aria-label')
      await act(async () => tile.click())
      assert.notEqual(tile.getAttribute('aria-label'), label)
      const hint = [
        ...rendered.container.querySelectorAll<HTMLButtonElement>('button'),
      ].find((button) => button.textContent?.includes('Hint'))
      assert.ok(hint)
      for (let move = 0; move < 76 && !hint.disabled; move++) {
        await act(async () => hint.click())
      }
      assert.equal(hint.disabled, true)
      assert.match(
        rendered.container.querySelector('[role="status"]')?.textContent ?? '',
        /Connected in/
      )
      const again = [
        ...rendered.container.querySelectorAll<HTMLButtonElement>('button'),
      ].find((button) => button.textContent?.includes('Play again'))
      assert.ok(again)
      await act(async () => again.click())
      assert.equal(hint.disabled, false)
      assert.equal(requests, 0)
      const active = rendered.container.querySelector<HTMLButtonElement>(
        '[role="grid"] button[tabindex="0"]'
      )
      assert.ok(active)
      await act(async () =>
        active.dispatchEvent(
          new domWindow.KeyboardEvent('keydown', {
            key: 'ArrowRight',
            bubbles: true,
          }) as unknown as Event
        )
      )
      assert.equal(
        rendered.container.querySelectorAll(
          '[role="grid"] button[tabindex="0"]'
        ).length,
        1
      )
    } finally {
      await act(async () => rendered.root.unmount())
    }
  })

  test('keeps settings collapsed without blocking play', async () => {
    const rendered = await renderArtwork()
    try {
      const settings = rendered.container.querySelector('details')
      assert.ok(settings)
      assert.equal(settings.open, false)
      assert.match(
        settings.querySelector('summary')?.textContent ?? '',
        /5 × 5/
      )
      assert.equal(settings.querySelectorAll('option').length, 3)
      assert.equal(
        rendered.container.querySelectorAll('[role="gridcell"]').length,
        25
      )
      const practice = rendered.container.querySelector(
        '.signal-game-modes button[aria-pressed="true"]'
      )
      assert.equal(practice?.textContent?.trim(), 'Practice')
      assert.equal(
        rendered.container
          .querySelector('.signal-game-link')
          ?.getAttribute('href'),
        '/games/signal'
      )
      const note = rendered.container.querySelector('.signal-game-note')
      assert.match(note?.textContent ?? '', /sign in or register at any time/)
    } finally {
      await act(async () => rendered.root.unmount())
    }
  })
})

test('settings follow published board size and session ownership without resetting the public game', async () => {
  login('signal-owner-a')
  const rendered = await renderArtwork()
  try {
    const firstSelect = rendered.container.querySelector('select')
    assert.ok(firstSelect)
    await act(async () => {
      firstSelect.value = '12'
      firstSelect.dispatchEvent(new Event('change', { bubbles: true }))
    })
    assert.equal(firstSelect.value, '12')
    await act(async () => startSignalGame('practice', 8))
    const select = rendered.container.querySelector('select')
    assert.ok(select)
    assert.equal(select.value, '8')
    await act(async () => {
      select.value = '12'
      select.dispatchEvent(new Event('change', { bubbles: true }))
    })
    await act(async () => startSignalGame('practice', 8))
    assert.equal(select.value, '12', 'same-size rounds retain the size draft')
    const round = useSignalGame.getState().roundId
    await act(async () => login('signal-owner-b'))
    assert.equal(rendered.container.querySelector('select')?.value, '8')
    assert.equal(useSignalGame.getState().roundId, round)
    assert.equal(
      rendered.container.querySelectorAll('[role=gridcell]').length,
      64
    )
  } finally {
    await act(async () => rendered.root.unmount())
  }
})

test('a cancelled challenge failure cannot mark the replacement practice round unavailable', async () => {
  let rejectOld: ((cause: Error) => void) | undefined
  let signal: AbortSignal | undefined
  api.post = (async (
    url: string,
    _body: unknown,
    options: { signal: AbortSignal }
  ) => {
    assert.equal(url, '/api/games/signal/attempts')
    signal = options.signal
    return await new Promise((_resolve, reject) => {
      rejectOld = reject
    })
  }) as typeof api.post
  const rendered = await renderArtwork()
  try {
    await act(async () =>
      control(rendered.container, 'Start challenge').click()
    )
    assert.ok(rejectOld)
    assert.equal(useSignalGame.getState().phase, 'loading')
    await act(async () => startSignalGame('practice', 5))
    const round = useSignalGame.getState().roundId
    assert.equal(signal?.aborted, true)
    await act(async () => {
      rejectOld?.(new Error('Old challenge failed'))
      await Promise.resolve()
    })
    assert.equal(useSignalGame.getState().roundId, round)
    assert.equal(useSignalGame.getState().phase, 'playing')
    assert.equal(useSignalGame.getState().serviceError, false)
    assert.equal(rendered.container.querySelector('[role=alert]'), null)
    assert.ok(
      [
        ...rendered.container.querySelectorAll<HTMLButtonElement>(
          '[role=grid] button'
        ),
      ].every((tile) => !tile.disabled)
    )
  } finally {
    await act(async () => rendered.root.unmount())
  }
})

test('an old-session delayed challenge receipt cannot replace a subsequently started practice circuit', async () => {
  login('signal-pending-a')
  let resolveOld: ((value: unknown) => void) | undefined
  let signal: AbortSignal | undefined
  api.post = (async (
    url: string,
    _body: unknown,
    options: { signal: AbortSignal }
  ) => {
    assert.equal(url, '/api/games/signal/attempts')
    signal = options.signal
    return await new Promise<unknown>((resolve) => {
      resolveOld = resolve
    })
  }) as typeof api.post
  const rendered = await renderArtwork()
  try {
    await act(async () =>
      control(rendered.container, 'Start challenge').click()
    )
    assert.ok(resolveOld)
    await act(async () => {
      login('signal-pending-b')
      await startSignalGame('practice', 5)
    })
    const current = useSignalGame.getState()
    assert.equal(signal?.aborted, true)
    await act(async () => {
      resolveOld?.({
        data: {
          success: true,
          data: {
            token: 'a'.repeat(43),
            seed: 123,
            size: 5,
            day: '2026-11-14',
            rules_version: RULES_VERSION,
            countdown_seconds: 3,
          },
        },
      })
      await Promise.resolve()
    })
    assert.equal(useSignalGame.getState().roundId, current.roundId)
    assert.equal(useSignalGame.getState().circuit.seed, current.circuit.seed)
    assert.equal(useSignalGame.getState().token, undefined)
    assert.equal(useSignalGame.getState().mode, 'practice')
    assert.equal(useSignalGame.getState().phase, 'playing')
    assert.equal(rendered.container.querySelector('[role=alert]'), null)
  } finally {
    await act(async () => rendered.root.unmount())
  }
})

test('a superseded failure stays silent while a new challenge is pending, but its current failure remains visible', async () => {
  const rejections: Array<(error: Error) => void> = []
  api.post = (async () =>
    await new Promise<unknown>((_resolve, reject) => {
      rejections.push(reject)
    })) as typeof api.post
  const rendered = await renderArtwork()
  let currentFailure: Promise<unknown> | undefined
  try {
    await act(async () =>
      control(rendered.container, 'Start challenge').click()
    )
    await act(async () => {
      currentFailure = startSignalGame('challenge', 5).then(
        () => null,
        (cause) => cause
      )
    })
    assert.equal(rejections.length, 2)
    await act(async () => {
      rejections[0](new Error('Superseded'))
      await Promise.resolve()
    })
    assert.equal(useSignalGame.getState().phase, 'loading')
    assert.equal(useSignalGame.getState().serviceError, false)
    assert.equal(rendered.container.querySelector('[role=alert]'), null)
    await act(async () => {
      rejections[1](new Error('Current challenge unavailable'))
      assert.ok((await currentFailure) instanceof Error)
    })
    assert.equal(useSignalGame.getState().serviceError, true)
    assert.ok(rendered.container.querySelector('[role=alert]'))
  } finally {
    await act(async () => {
      await startSignalGame('practice', 5)
      rendered.root.unmount()
    })
  }
})
