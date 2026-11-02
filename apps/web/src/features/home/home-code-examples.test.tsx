/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { after, test } from 'node:test'

import { Window } from 'happy-dom'

import { resolveApiBaseUrl } from '@/lib/api-base-url'

import { CODE_TABS, codeForTab } from './home-code-examples'

test('all examples use the resolved domain, port and prefix without shell expansion', () => {
  for (const address of [
    undefined,
    'https://relay.example.test:8443/gateway/v1/',
    "https://relay.example.test/$(printf%20EXPANDED)/o'clock",
  ]) {
    const base = resolveApiBaseUrl(address, 'http://127.0.0.1:3210')
    assert.ok(base)
    const root = base.replace(/\/v1$/, '')
    for (const tab of CODE_TABS) {
      const code = codeForTab(tab, base)
      assert.equal(code.includes('api.lmm.best'), false)
      if (tab === 'API') {
        const quoted = code.match(/baseURL: (".*"),/)?.[1]
        assert.ok(quoted)
        assert.equal(JSON.parse(quoted), base)
      } else {
        // Print the shell's actual arguments; no curl or network request runs.
        const args = execFileSync(
          '/bin/sh',
          ['-c', code.replace(/^curl /, 'printf "%s\\n" ')],
          {
            encoding: 'utf8',
            env: { LMM_API_KEY: 'synthetic-local-placeholder' },
          }
        )
        const endpoint =
          tab === 'Claude'
            ? '/v1/messages'
            : tab === 'Gemini'
              ? '/v1beta/models/model-name:generateContent'
              : '/v1/chat/completions'
        assert.equal(
          args.split('\n').filter((arg) => arg === root + endpoint).length,
          1
        )
      }
    }
  }
})

const dom = new Window()
const previous = new Map<string, PropertyDescriptor | undefined>()
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
  previous.set(key, Object.getOwnPropertyDescriptor(globalThis, key))
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: dom[key],
  })
}
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { CodePreview } = await import('./home-code-preview')
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
after(() => {
  dom.close()
  for (const [key, descriptor] of previous) {
    if (descriptor) Object.defineProperty(globalThis, key, descriptor)
    else Reflect.deleteProperty(globalThis, key)
  }
})

test('preview renders the supplied snippet and disables copying when its address is unavailable', async () => {
  const container = document.body.appendChild(document.createElement('div'))
  const root = createRoot(container)
  let copied = 0
  const render = async (code: string, loading = false) => {
    await act(async () =>
      root.render(
        <CodePreview
          t={(key) => key}
          tab='Chat'
          code={code}
          loading={loading}
          copied={false}
          onTabChange={() => {}}
          onCopy={() => {
            copied++
          }}
        />
      )
    )
    const button = container.querySelector<HTMLButtonElement>(
      'button[aria-label="Copy"]'
    )
    assert.ok(button)
    return button
  }
  try {
    for (const loading of [true, false]) {
      const button = await render('', loading)
      assert.equal(button.disabled, true)
      button.click()
      assert.equal(copied, 0)
      assert.equal(
        container.querySelector('pre code')?.textContent,
        loading ? 'Loading...' : 'API address unavailable'
      )
    }
    const code = codeForTab('Chat', 'https://tenant.example.test/edge/v1')
    const button = await render(code)
    assert.equal(button.disabled, false)
    assert.equal(container.querySelector('pre code')?.textContent, code)
    button.click()
    assert.equal(copied, 1)
  } finally {
    await act(async () => root.unmount())
    container.remove()
  }
})
