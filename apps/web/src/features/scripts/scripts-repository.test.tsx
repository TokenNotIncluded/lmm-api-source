/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, test } from 'node:test'

import { Window } from 'happy-dom'

const dom = new Window({ url: 'https://console.example.test/settings/scripts' })
dom.document.write('<!doctype html><html><head></head><body></body></html>')
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLInputElement',
  'SVGElement',
  'Node',
  'Element',
  'Event',
  'MouseEvent',
  'CustomEvent',
  'MutationObserver',
  'ResizeObserver',
  'requestAnimationFrame',
  'cancelAnimationFrame',
  'getComputedStyle',
  'localStorage',
  'sessionStorage',
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
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { ScriptsAdminSection } = await import('./scripts-panel')
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
after(() => dom.close())
const flush = () => new Promise((resolve) => setTimeout(resolve, 30))

test('repository edits preserve the read revision and pull only the saved configuration', async () => {
  const original = api.defaults.adapter
  const calls: { method: string | undefined; data: unknown }[] = []
  let revision = 4
  let branch = 'main'
  api.defaults.adapter = async (config) => {
    let data: unknown
    if (config.url === '/api/scripts') {
      data = { success: true, data: [] }
    } else {
      assert.ok(config.url?.startsWith('/api/scripts/repository'))
      if (config.method !== 'get') {
        const body = config.data ? JSON.parse(String(config.data)) : null
        calls.push({ method: config.method, data: body })
        if (config.method === 'put') {
          assert.equal(
            body.revision,
            4,
            'keep the revision that the draft started from'
          )
          branch = body.branch
        }
        revision++
      }
      data = {
        success: true,
        data: {
          revision,
          repository_url: 'https://github.com/example/scripts',
          branch,
          github_key_set: false,
        },
      }
    }
    return { config, data, headers: {}, status: 200, statusText: 'OK' }
  }
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const container = document.body.appendChild(document.createElement('div'))
  const root = createRoot(container)
  const button = (label: string) => {
    const found = [...container.querySelectorAll('button')].find(
      (node) => node.textContent?.trim() === label
    )
    assert.ok(found, label)
    return found
  }
  try {
    await act(async () => {
      root.render(
        <I18nextProvider i18n={i18n}>
          <QueryClientProvider client={client}>
            <ScriptsAdminSection />
          </QueryClientProvider>
        </I18nextProvider>
      )
      await flush()
    })
    await act(async () => {
      await flush()
    })
    assert.equal(button('Pull updates').disabled, false)
    const input = container.querySelector<HTMLInputElement>(
      '#scripts-repository-branch'
    )
    assert.ok(input)
    const setter = Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      'value'
    )?.set
    assert.ok(setter)
    await act(async () => {
      setter.call(input, 'release')
      input.dispatchEvent(new Event('input', { bubbles: true }))
      await flush()
    })
    assert.equal(button('Pull updates').disabled, true)
    // A background refresh must not silently approve overwriting another admin.
    revision = 5
    await act(async () => {
      await client.invalidateQueries({ queryKey: ['scripts-repository'] })
      await flush()
    })
    await act(async () => {
      button('Save repository').click()
      await flush()
    })
    await act(async () => {
      await flush()
    })
    assert.deepEqual(calls, [
      {
        method: 'put',
        data: {
          revision: 4,
          repository_url: 'https://github.com/example/scripts',
          branch: 'release',
          github_key: '',
          clear_github_key: false,
        },
      },
    ])
    assert.equal(button('Pull updates').disabled, false)
    await act(async () => {
      button('Pull updates').click()
      await flush()
    })
    assert.equal(calls.length, 2)
    assert.equal(calls[1].method, 'post')
  } finally {
    await act(async () => root.unmount())
    client.clear()
    container.remove()
    api.defaults.adapter = original
  }
})
