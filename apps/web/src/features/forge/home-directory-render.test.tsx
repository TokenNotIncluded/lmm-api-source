/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { after, test } from 'node:test'

import { Window } from 'happy-dom'

const dom = new Window({ url: 'https://console.example.test/' })
const previous = new Map<string, PropertyDescriptor | undefined>()
for (const key of [
  'window',
  'document',
  'navigator',
  'location',
  'HTMLElement',
  'Node',
  'Element',
  'Event',
  'MouseEvent',
  'WheelEvent',
  'requestAnimationFrame',
  'cancelAnimationFrame',
  'getComputedStyle',
  'scrollTo',
] as const) {
  previous.set(key, Object.getOwnPropertyDescriptor(globalThis, key))
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: dom[key],
  })
}
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { createRootRoute, createRouter, createMemoryHistory, RouterProvider } =
  await import('@tanstack/react-router')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { HomeDirectoryLink } = await import('./home-directory-link')
const resources = Object.fromEntries(
  ['en', 'zh-CN', 'zh-TW', 'fr', 'ja', 'ru', 'vi'].map((locale) => [
    locale,
    JSON.parse(
      readFileSync(
        new URL(`../../i18n/locales/${locale}.json`, import.meta.url),
        'utf8'
      )
    ),
  ])
)
const i18n = createInstance()
await i18n.use(initReactI18next).init({ lng: 'en', resources })
const flush = () => new Promise((resolve) => setTimeout(resolve, 30))
after(() => {
  dom.close()
  for (const [key, descriptor] of previous) {
    if (descriptor) Object.defineProperty(globalThis, key, descriptor)
    else Reflect.deleteProperty(globalThis, key)
  }
})

test('directory entry uses the active locale and displays progress only during a real gesture', async () => {
  const router = createRouter({
    routeTree: createRootRoute({ component: HomeDirectoryLink }),
    history: createMemoryHistory({ initialEntries: ['/'] }),
  })
  const container = document.body.appendChild(document.createElement('div'))
  const root = createRoot(container)
  try {
    await act(async () => {
      root.render(
        <I18nextProvider i18n={i18n}>
          <RouterProvider router={router} />
        </I18nextProvider>
      )
      await flush()
    })
    await act(flush)
    const entry = container.querySelector<HTMLElement>('aside')
    assert.ok(entry)
    assert.equal(
      entry.querySelector('a')?.getAttribute('href'),
      '/ai-directory'
    )
    assert.equal(entry.querySelector('.home-directory-percent'), null)
    for (const locale of Object.keys(resources)) {
      await act(async () => {
        await i18n.changeLanguage(locale)
      })
      assert.equal(
        entry.querySelector('.home-directory-wheel-hint')?.textContent,
        resources[locale].translation['Scroll up to explore']
      )
      assert.equal(
        entry.querySelector('.home-directory-touch-hint')?.textContent,
        resources[locale].translation['Swipe up here to explore']
      )
    }
    entry.getBoundingClientRect = () => ({
      top: 0,
      bottom: 80,
      left: 0,
      right: 390,
      width: 390,
      height: 80,
      x: 0,
      y: 0,
      toJSON: () => ({}),
    })
    await act(async () => {
      entry.dispatchEvent(
        new WheelEvent('wheel', {
          deltaY: -80,
          bubbles: true,
          cancelable: true,
        })
      )
    })
    assert.equal(
      entry.querySelector('.home-directory-percent')?.textContent,
      '33%'
    )
    assert.equal(
      entry
        .querySelector('[role="progressbar"]')
        ?.getAttribute('aria-valuenow'),
      '33'
    )
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 850))
    })
    assert.equal(entry.querySelector('.home-directory-percent'), null)
  } finally {
    await act(async () => root.unmount())
    container.remove()
  }
})
