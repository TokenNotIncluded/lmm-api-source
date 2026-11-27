/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
// @ts-expect-error Bun's test module is available only in the test runtime.
import { mock } from 'bun:test'
import assert from 'node:assert/strict'
import { after, test } from 'node:test'

import { Window } from 'happy-dom'

import type { NativeConfiguration } from '@/features/channels/native-api'

import { writeOfficialToml } from './official-price-toml'

const dom = new Window()
for (const [key, value] of Object.entries({
  window: dom,
  document: dom.document,
  navigator: dom.navigator,
  HTMLElement: dom.HTMLElement,
  Event: dom.Event,
  IS_REACT_ACT_ENVIRONMENT: true,
})) {
  Object.defineProperty(globalThis, key, { value, configurable: true })
}
after(() => dom.happyDOM.abort())
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })

const name = 'vendor/model'
const initial = 'model = "vendor/model"\nsource = "manual"'
const zero = { numerator: 0, denominator: 1 }
const partial = writeOfficialToml(name, {
  source: 'channel/31',
  source_unit: 'USD/1M tokens',
  billing_mode: 'tiered_decimal',
  rates: { input: 0, output: 0, cache_read: 0, cache_write: 0 },
  tiered: {
    source_digest: 'a'.repeat(64),
    revision: 2,
    uses_cache_read: true,
    uses_cache_write: true,
    tiers: [
      {
        rates: {
          input: zero,
          output: zero,
          cache_read: zero,
          cache_write: zero,
        },
      },
    ],
  },
})
const fetched = writeOfficialToml(name, {
  source: 'channel/31',
  source_unit: 'USD/1M tokens',
  rates: { input: 100, output: 200, cache_read: 0, cache_write: 0, request: 0 },
})
let imported = partial
const receipt = {
  changes: { added: [], modified: [name], deleted: [] },
  digest: 'reviewed',
}
let found = false
const published: unknown[] = []
const proposals: unknown[] = []
mock.module('./official-price-api', () => ({
  readOfficialPriceToml: async () => initial,
  syncOfficialPriceToml: async (
    _scope: unknown,
    model: string,
    _draft: string,
    source: unknown
  ) => {
    assert.equal(model, name)
    assert.deepEqual(source, {
      kind: 'channel',
      channelId: 31,
      upstreamModel: 'provider/model',
    })
    return { found, toml: found ? imported : '' }
  },
  previewOfficialPriceToml: async (
    _scope: unknown,
    _config: unknown,
    model: string,
    toml: string | null
  ) => {
    proposals.push([model, toml])
    return toml === null
      ? { changes: { added: [], modified: [], deleted: [model] } }
      : receipt
  },
  publishOfficialPrice: async (_scope: unknown, proposal: unknown) =>
    published.push(proposal),
}))
const { OfficialPriceEditor } = await import('./official-price-editor')

test('channel quotes remain editable through separate diff and publish steps; stale removal cannot publish', async () => {
  const scope = { userId: 2, sessionId: 'admin', role: 100 }
  const config = {
    version: { routing: 2, pricing: 5 },
    routing: {
      routes: [],
      upstreams: [{ id: 31, protocol: 'openai_responses' }],
    },
    bindings: {
      names: { 31: 'Supplier' },
      official_prices: { models: { [name]: {} } },
    },
  } as unknown as NativeConfiguration
  const host = document.createElement('div')
  document.body.append(host)
  const client = new QueryClient()
  const root = createRoot(host)
  let refreshes = 0
  const render = async (configuration: NativeConfiguration) => {
    await act(async () =>
      root.render(
        <I18nextProvider i18n={i18n}>
          <QueryClientProvider client={client}>
            <OfficialPriceEditor
              scope={scope}
              configuration={configuration}
              model={name}
              channelId={31}
              upstreamModel='provider/model'
              onPublished={() => {
                refreshes++
              }}
            />
          </QueryClientProvider>
        </I18nextProvider>
      )
    )
  }
  const button = (label: string) =>
    [...host.querySelectorAll('button')].find((el) => el.textContent === label)
  const click = async (label: string) => {
    const target = button(label)
    assert.ok(target && !target.disabled, `expected enabled ${label}`)
    await act(async () => target.click())
  }
  try {
    await render(config)
    for (
      let count = 0;
      !host.querySelector('textarea') && count < 50;
      count++
    ) {
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 5))
      })
    }
    await click('Fetch upstream prices')
    assert.equal(host.querySelector('textarea')?.value, initial)
    assert.ok(host.textContent?.includes('No matching upstream prices'))
    assert.ok(button('models.dev'))
    found = true
    await click('Fetch upstream prices')
    assert.equal(host.querySelector('textarea')?.value, partial)
    assert.ok(host.textContent?.includes('Missing price fields'))
    assert.ok(host.textContent?.includes('Set tiered.revision to 4'))
    assert.deepEqual(
      [...host.querySelectorAll('li code')].map((el) => el.textContent),
      ['rates.request', 'tiered.tiers[0].rates.cache_write_12h']
    )
    assert.equal(button('Preview changes')?.disabled, true)
    assert.deepEqual(proposals, [])
    imported = fetched
    await click('Fetch upstream prices')
    assert.equal(host.textContent?.includes('Missing price fields'), false)
    assert.equal(host.querySelector('textarea')?.value, fetched)
    assert.equal(button('Publish prices'), undefined)
    await click('Preview changes')
    assert.deepEqual(proposals, [[name, fetched]])
    assert.deepEqual(published, [])
    assert.ok(host.querySelector('dl')?.textContent?.includes(`Updated${name}`))
    await click('Publish prices')
    assert.deepEqual(published, [receipt])
    assert.equal(refreshes, 1)
    await render({ ...config, version: { routing: 3, pricing: 6 } })
    await click('Reload saved version')
    for (
      let count = 0;
      !button('Preview price removal') ||
      button('Preview price removal')?.disabled;
      count++
    ) {
      assert.ok(count < 50, 'expected fresh configuration')
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 5))
      })
    }
    await click('Preview price removal')
    assert.deepEqual(proposals[1], [name, null])
    assert.ok(host.querySelector('dl')?.textContent?.includes(`Removed${name}`))
    await render({ ...config, version: { routing: 4, pricing: 7 } })
    assert.equal(button('Publish prices'), undefined)
    assert.equal(host.querySelector('textarea')?.readOnly, true)
    assert.deepEqual(published, [receipt])
  } finally {
    await act(async () => root.unmount())
    client.clear()
    host.remove()
  }
})
