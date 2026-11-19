/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { after, beforeEach, test } from 'node:test'

import { Window } from 'happy-dom'

const window = new Window({ url: 'https://console.example.test/' })
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'Node',
  'Element',
  'Event',
  'MutationObserver',
  'localStorage',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: key === 'window' ? window : window[key],
  })
}
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { ThemeProvider, useTheme } = await import('./theme-provider')
const { ThemeCustomizationProvider, useThemeCustomization } =
  await import('./theme-customization-provider')
const { getCookie, removeCookie, setCookie } = await import('@/lib/cookies')
const { THEME_COOKIE_KEYS, THEME_PRESETS } =
  await import('@/lib/theme-customization')
const bootstrap = readFileSync(
  new URL('../../index.html', import.meta.url),
  'utf8'
).match(/<script id="theme-customization-bootstrap">([\s\S]*?)<\/script>/)?.[1]
assert.ok(bootstrap)
const applyBootstrap = new Function('document', bootstrap)
beforeEach(() => {
  for (const name of [
    ...Object.values(THEME_COOKIE_KEYS),
    'theme_preset_default_migrated_v1',
    'vite-ui-theme',
  ]) {
    removeCookie(name)
  }
  for (const name of document.body.getAttributeNames()) {
    if (name.startsWith('data-theme-')) document.body.removeAttribute(name)
  }
})
after(() => window.happyDOM.abort())

async function render() {
  let theme: ReturnType<typeof useTheme> | undefined
  let style: ReturnType<typeof useThemeCustomization> | undefined
  function Harness() {
    theme = useTheme()
    style = useThemeCustomization()
    return <span>{theme.resolvedTheme}</span>
  }
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  await act(async () => {
    root.render(
      <ThemeProvider defaultTheme='light'>
        <ThemeCustomizationProvider>
          <Harness />
        </ThemeCustomizationProvider>
      </ThemeProvider>
    )
  })
  return {
    current: () => {
      assert.ok(theme)
      assert.ok(style)
      return { theme, style }
    },
    close: async () => {
      await act(async () => root.unmount())
      container.remove()
    },
  }
}

test('appearance preview changes real provider output without saving cookies', async () => {
  // Happy DOM may briefly expose the empty preset cookie marked for deletion.
  // Compare persistent values while retaining all other cookie assertions.
  const persistentCookies = () =>
    document.cookie
      .split('; ')
      .filter((cookie) => cookie !== 'theme_preset=')
      .join('; ')
  const mounted = await render()
  try {
    await act(async () => {
      mounted.current().theme.setTheme('light')
      mounted.current().style.setPreset('default')
    })
    const cookies = persistentCookies()
    let stopMode!: () => void
    let stopPreset!: () => void
    await act(async () => {
      stopMode = mounted.current().theme.previewTheme('dark')
      stopPreset = mounted.current().style.previewPreset('rose-garden')
    })
    assert.equal(document.documentElement.classList.contains('dark'), true)
    assert.equal(document.body.getAttribute('data-theme-preset'), 'rose-garden')
    assert.equal(mounted.current().theme.resolvedTheme, 'dark')
    assert.equal(mounted.current().theme.theme, 'light')
    assert.equal(mounted.current().style.customization.preset, 'default')
    assert.equal(persistentCookies(), cookies)
    await act(async () => {
      stopMode()
      stopPreset()
    })
    assert.equal(document.documentElement.classList.contains('light'), true)
    assert.equal(document.body.getAttribute('data-theme-preset'), 'default')
    assert.equal(persistentCookies(), cookies)
  } finally {
    await mounted.close()
  }
})

test('manual choices, including the same saved value, win over old preview cleanup', async () => {
  const mounted = await render()
  try {
    await act(async () => {
      mounted.current().theme.setTheme('light')
      mounted.current().style.setPreset('default')
    })
    let stopMode!: () => void
    let stopPreset!: () => void
    await act(async () => {
      stopMode = mounted.current().theme.previewTheme('dark')
      stopPreset = mounted.current().style.previewPreset('rose-garden')
    })
    await act(async () => {
      mounted.current().theme.setTheme('light')
      mounted.current().style.setPreset('ocean-breeze')
    })
    await act(async () => {
      stopMode()
      stopPreset()
    })
    assert.equal(mounted.current().theme.resolvedTheme, 'light')
    assert.equal(
      document.body.getAttribute('data-theme-preset'),
      'ocean-breeze'
    )
    assert.equal(getCookie('theme_preset'), 'ocean-breeze')
  } finally {
    await mounted.close()
  }
})

test('Classic is the default palette before the first paint and after mounting', async () => {
  applyBootstrap(document)
  assert.equal(document.body.dataset.themePreset, 'default')
  assert.equal(document.body.dataset.themeFont, 'sans')
  const mounted = await render()
  try {
    assert.equal(mounted.current().style.customization.preset, 'default')
    assert.equal(mounted.current().style.defaults.preset, 'default')
    assert.equal(document.body.dataset.themePreset, 'default')
    assert.equal(document.body.dataset.themeFont, 'sans')
  } finally {
    await mounted.close()
  }
})

test('first-paint initialization preserves every registered palette and explicit axes', () => {
  setCookie('theme_font', 'sans')
  setCookie('theme_radius', 'xl')
  setCookie('theme_scale', 'lg')
  setCookie('theme_content_layout', 'centered')
  for (const { value } of THEME_PRESETS) {
    setCookie('theme_preset', value)
    applyBootstrap(document)
    assert.equal(document.body.dataset.themePreset, value)
    assert.equal(document.body.dataset.themeFont, 'sans')
    assert.equal(document.body.dataset.themeRadius, 'xl')
    assert.equal(document.body.dataset.themeScale, 'lg')
    assert.equal(document.body.dataset.themeContentLayout, 'centered')
  }
})

test('legacy warm-palette cookies migrate to Ant without losing the saved font', async () => {
  setCookie('theme_preset', 'anthropic')
  setCookie('theme_preset_default_migrated_v1', '1')
  setCookie('theme_font', 'sans')
  applyBootstrap(document)
  assert.equal(document.body.dataset.themePreset, 'ant')
  assert.equal(document.body.dataset.themeFont, 'sans')
  const mounted = await render()
  assert.equal(mounted.current().style.customization.preset, 'ant')
  assert.equal(getCookie('theme_preset'), 'ant')
  await mounted.close()
  const reloaded = await render()
  try {
    assert.equal(reloaded.current().style.customization.preset, 'ant')
    assert.equal(reloaded.current().style.customization.font, 'sans')
    assert.equal(document.body.dataset.themePreset, 'ant')
    assert.equal(document.body.dataset.themeFont, 'sans')
  } finally {
    await reloaded.close()
  }
})

test('appearance choices survive remount and reset restores Classic and all layout defaults', async () => {
  const mounted = await render()
  await act(async () => {
    const style = mounted.current().style
    style.setPreset('ocean-breeze')
    style.setFont('sans')
    style.setRadius('sm')
    style.setScale('lg')
    style.setContentLayout('centered')
  })
  await mounted.close()
  const reloaded = await render()
  try {
    assert.deepEqual(reloaded.current().style.customization, {
      preset: 'ocean-breeze',
      font: 'sans',
      radius: 'sm',
      scale: 'lg',
      contentLayout: 'centered',
    })
    assert.equal(document.body.dataset.themeRadius, 'sm')
    assert.equal(document.body.dataset.themeScale, 'lg')
    assert.equal(document.body.dataset.themeContentLayout, 'centered')
    await act(async () => reloaded.current().style.resetCustomization())
    assert.deepEqual(
      reloaded.current().style.customization,
      reloaded.current().style.defaults
    )
    assert.equal(document.body.dataset.themePreset, 'default')
    assert.equal(document.body.dataset.themeFont, 'sans')
    assert.equal(document.body.getAttribute('data-theme-radius'), null)
    assert.equal(document.body.getAttribute('data-theme-scale'), null)
    assert.equal(document.body.dataset.themeContentLayout, 'full')
    for (const key of Object.values(THEME_COOKIE_KEYS)) {
      assert.ok(!getCookie(key))
    }
  } finally {
    await reloaded.close()
  }
})
