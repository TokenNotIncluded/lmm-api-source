/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import assert from 'node:assert/strict'
import { after, test } from 'node:test'

import { Window } from 'happy-dom'

const dom = new Window({ url: 'http://localhost/' })
dom.document.write('<!doctype html><html><body></body></html>')
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'Element',
  'Node',
  'Event',
  'MutationObserver',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: key === 'window' ? dom : dom[key],
  })
}
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const { createInstance } = await import('i18next')
const { initReactI18next } = await import('react-i18next')
await createInstance()
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
const { DevelopersPage, IntegrationPrompt } = await import('./page')
const { buildPricingExample, buildPricingPrompt, OAUTH_PROMPT } =
  await import('./integration-prompts')
const publicRoot = 'https://relay.example.test:8443/gateway'
const PRICING_EXAMPLE = buildPricingExample(publicRoot)
const PRICING_PROMPT = buildPricingPrompt(publicRoot)
after(() => dom.close())

test('one click copies the complete pricing integration instructions', async () => {
  let copied = ''
  Object.defineProperty(navigator, 'clipboard', {
    configurable: true,
    value: {
      writeText: async (value: string) => {
        copied = value
      },
    },
  })
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  try {
    await act(async () =>
      root.render(
        <IntegrationPrompt
          prompt={PRICING_PROMPT}
          label='Copy pricing prompt'
        />
      )
    )
    const copyButton = container.querySelector('button')
    assert.ok(copyButton)
    await act(async () => copyButton.click())
    assert.equal(copied, PRICING_PROMPT)
    assert.equal(container.querySelector('button')?.textContent, 'Copied')
    assert.equal(container.querySelector('[role="alert"]'), null)
  } finally {
    await act(async () => root.unmount())
    container.remove()
  }
})

test('blocked clipboard opens and selects the OAuth prompt instead of claiming success', async () => {
  Object.defineProperty(navigator, 'clipboard', {
    configurable: true,
    value: undefined,
  })
  Object.defineProperty(document, 'execCommand', {
    configurable: true,
    value: () => false,
  })
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  try {
    await act(async () =>
      root.render(
        <IntegrationPrompt prompt={OAUTH_PROMPT} label='Copy OAuth prompt' />
      )
    )
    const copyButton = container.querySelector('button')
    assert.ok(copyButton)
    await act(async () => copyButton.click())
    const field = container.querySelector('textarea')
    assert.ok(field)
    assert.equal(container.querySelector('details')?.open, true)
    assert.equal(field.value, OAUTH_PROMPT)
    assert.equal(field.selectionStart, 0)
    assert.equal(field.selectionEnd, OAUTH_PROMPT.length)
    assert.equal(
      container.querySelector('button')?.textContent,
      'Copy OAuth prompt'
    )
    assert.ok(container.querySelector('[role="alert"]'))
  } finally {
    await act(async () => root.unmount())
    container.remove()
  }
})

for (const clipboardAvailable of [true, false]) {
  test(`pricing example preserves the complete source and handles ${clipboardAvailable ? 'copy success' : 'blocked clipboard'}`, async () => {
    let copied = ''
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: clipboardAvailable
        ? {
            writeText: async (value: string) => {
              copied = value
            },
          }
        : undefined,
    })
    Object.defineProperty(document, 'execCommand', {
      configurable: true,
      value: () => false,
    })
    const queryClient = new QueryClient({
      defaultOptions: { queries: { enabled: false } },
    })
    queryClient.setQueryData(['status', 'anonymous'], {
      server_address: `${publicRoot}/v1`,
    })
    const container = document.createElement('div')
    document.body.append(container)
    const root = createRoot(container)
    try {
      await act(async () =>
        root.render(
          <QueryClientProvider client={queryClient}>
            <DevelopersPage />
          </QueryClientProvider>
        )
      )
      const example = container.querySelector<HTMLPreElement>(
        'pre[aria-label="Fetch pricing from your backend."]'
      )
      assert.ok(example)
      const builder = container.querySelector('.dev-builder-output')
      assert.ok(
        builder?.textContent?.includes(`${publicRoot}/v1/chat/completions`)
      )
      assert.ok(
        container.querySelector(
          'a[href="https://api.lmm.best/.well-known/oauth-authorization-server"]'
        )
      )
      assert.equal(example.textContent, PRICING_EXAMPLE)
      const exampleBlock = example.parentElement
      assert.ok(exampleBlock)
      const copyButton = exampleBlock.querySelector('button')
      assert.ok(copyButton)
      await act(async () => copyButton.click())
      if (clipboardAvailable) {
        assert.equal(copied, PRICING_EXAMPLE)
        assert.equal(copyButton.textContent, 'Copied')
        assert.equal(exampleBlock.querySelector('[role="alert"]'), null)
      } else {
        assert.equal(copied, '')
        assert.equal(copyButton.textContent, 'Copy')
        assert.equal(
          exampleBlock.querySelector('[role="alert"]')?.textContent,
          'Copy failed'
        )
        assert.equal(example.textContent, PRICING_EXAMPLE)
      }
    } finally {
      await act(async () => root.unmount())
      queryClient.clear()
      container.remove()
    }
  })
}

test('invalid configured API address disables request copying and hides pricing artifacts', async () => {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { enabled: false } },
  })
  queryClient.setQueryData(['status', 'anonymous'], {
    server_address: 'https://key:secret@relay.test',
  })
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  try {
    await act(async () =>
      root.render(
        <QueryClientProvider client={queryClient}>
          <DevelopersPage />
        </QueryClientProvider>
      )
    )
    assert.match(container.textContent ?? '', /API address unavailable/)
    assert.doesNotMatch(container.textContent ?? '', /key:secret/)
    assert.equal(
      container.querySelector<HTMLButtonElement>('.dev-builder-output button')
        ?.disabled,
      true
    )
    assert.equal(
      container.querySelector(
        'pre[aria-label="Fetch pricing from your backend."]'
      ),
      null
    )
    assert.equal(
      container.querySelector(
        'textarea[aria-label="Integration prompt: Copy pricing prompt"]'
      ),
      null
    )
    assert.ok(
      container.querySelector(
        'a[href="https://api.lmm.best/.well-known/oauth-authorization-server"]'
      )
    )
  } finally {
    await act(async () => root.unmount())
    queryClient.clear()
    container.remove()
  }
})
