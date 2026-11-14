/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, test } from 'node:test'

import { Window } from 'happy-dom'

import type { Row } from '@/lib/table'

const dom = new Window()
for (const [name, value] of Object.entries({
  window: dom,
  document: dom.document,
  navigator: dom.navigator,
  HTMLElement: dom.HTMLElement,
  Node: dom.Node,
  Element: dom.Element,
  Event: dom.Event,
  MouseEvent: dom.MouseEvent,
})) {
  Object.defineProperty(globalThis, name, { configurable: true, value })
}
Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
  configurable: true,
  value: true,
})
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { DataTableRow } = await import('./data-table-row')

after(() => dom.happyDOM.close())

test('stable rows update and remove forwarded handlers, style, and ARIA props', async () => {
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  const row = {
    getVisibleCells: () => [],
    getIsSelected: () => false,
  } as unknown as Row<{ id: number }>
  const calls: string[] = []
  try {
    await act(async () => {
      root.render(
        <table>
          <tbody>
            <DataTableRow
              row={row}
              aria-label='Initial row'
              style={{ color: 'red' }}
              onClick={() => calls.push('old')}
            />
          </tbody>
        </table>
      )
    })
    await act(async () => {
      root.render(
        <table>
          <tbody>
            <DataTableRow
              row={row}
              aria-label='Updated row'
              style={{ color: 'blue' }}
              onClick={() => calls.push('new')}
            />
          </tbody>
        </table>
      )
    })
    const element = container.querySelector('tr')
    assert.ok(element)
    await act(async () => element.click())
    assert.deepEqual(calls, ['new'])
    assert.equal(element.style.color, 'blue')
    assert.equal(element.getAttribute('aria-label'), 'Updated row')

    await act(async () => {
      root.render(
        <table>
          <tbody>
            <DataTableRow row={row} />
          </tbody>
        </table>
      )
    })
    await act(async () => element.click())
    assert.deepEqual(calls, ['new'])
    assert.equal(element.style.color, '')
    assert.equal(element.hasAttribute('aria-label'), false)
  } finally {
    await act(async () => root.unmount())
    container.remove()
  }
})
