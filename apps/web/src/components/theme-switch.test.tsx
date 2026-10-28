/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.
*/
import assert from 'node:assert/strict'
import { after, describe, test } from 'node:test'

import { Window } from 'happy-dom'

const domWindow = new Window()
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'Node',
  'Element',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: domWindow[key],
  })
}

let systemPrefersDark = false

Object.defineProperty(window, 'matchMedia', {
  configurable: true,
  value: () => ({
    addEventListener: () => undefined,
    get matches() {
      return systemPrefersDark
    },
    removeEventListener: () => undefined,
  }),
})

const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { AccessRestrictionNotice } = await import('./access-restriction-notice')
const { ThemeProvider } = await import('@/context/theme-provider')
const { ThemeSwitch } = await import('./theme-switch')

const reactTestGlobals = globalThis as typeof globalThis & {
  IS_REACT_ACT_ENVIRONMENT?: boolean
}
reactTestGlobals.IS_REACT_ACT_ENVIRONMENT = true

describe('ThemeSwitch', () => {
  after(() => domWindow.close())

  test('toggles directly between light and dark themes', async () => {
    const container = document.createElement('div')
    const root = createRoot(container)

    await act(async () =>
      root.render(
        <ThemeProvider defaultTheme='light' storageKey='theme-switch-test'>
          <ThemeSwitch />
        </ThemeProvider>
      )
    )

    const toggle = container.querySelector(
      '[aria-label="Toggle theme"]'
    ) as HTMLButtonElement | null
    assert.ok(toggle)

    await act(async () => toggle.click())

    assert.equal(document.documentElement.classList.contains('dark'), true)
    assert.ok(container.querySelector('[aria-label="Theme options"]'))

    await act(async () => root.unmount())
  })

  test('uses the resolved system theme for the browser theme color', async () => {
    systemPrefersDark = true
    const metaThemeColor = document.createElement('meta')
    metaThemeColor.name = 'theme-color'
    document.head.append(metaThemeColor)
    const container = document.createElement('div')
    const root = createRoot(container)

    await act(async () =>
      root.render(
        <ThemeProvider
          defaultTheme='system'
          storageKey='theme-switch-system-test'
        >
          <ThemeSwitch />
        </ThemeProvider>
      )
    )

    assert.equal(metaThemeColor.content, '#020817')
    assert.equal(document.documentElement.classList.contains('dark'), true)

    await act(async () => root.unmount())
    metaThemeColor.remove()
    systemPrefersDark = false
  })
})

describe('AccessRestrictionNotice', () => {
  test('limits the notice to CN without making availability claims elsewhere', async () => {
    const container = document.createElement('div')
    const root = createRoot(container)

    await act(async () => root.render(<AccessRestrictionNotice />))

    assert.match(
      container.textContent ?? '',
      /only to ISO 3166-1 alpha-2 CN \(Mainland China\)/
    )
    assert.match(
      container.textContent ?? '',
      /does not state service availability for any other location/
    )

    await act(async () => root.unmount())
  })
})
