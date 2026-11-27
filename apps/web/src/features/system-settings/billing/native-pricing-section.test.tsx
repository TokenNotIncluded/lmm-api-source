/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
// @ts-expect-error Bun's test module is available only in the test runtime.
import { mock as moduleMock } from 'bun:test'
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { Window } from 'happy-dom'
import type { ReactNode } from 'react'

const dom = new Window()
for (const [name, value] of Object.entries({
  window: dom,
  document: dom.document,
  navigator: dom.navigator,
  HTMLElement: dom.HTMLElement,
  Element: dom.Element,
  Node: dom.Node,
  Event: dom.Event,
  MouseEvent: dom.MouseEvent,
  MutationObserver: dom.MutationObserver,
  ResizeObserver: dom.ResizeObserver,
  HTMLInputElement: dom.HTMLInputElement,
  HTMLButtonElement: dom.HTMLButtonElement,
  DocumentFragment: dom.DocumentFragment,
  ShadowRoot: dom.ShadowRoot,
  requestAnimationFrame: dom.requestAnimationFrame.bind(dom),
  cancelAnimationFrame: dom.cancelAnimationFrame.bind(dom),
  getComputedStyle: dom.getComputedStyle.bind(dom),
})) {
  Object.defineProperty(globalThis, name, { configurable: true, value })
}
Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
  configurable: true,
  value: true,
})

let section = 'model-pricing'
const router = await import('@tanstack/react-router')
moduleMock.module('@tanstack/react-router', () => ({
  ...router,
  useParams: () => ({ section }),
}))
moduleMock.module('../components/settings-page', () => ({
  SettingsPage: () => {
    throw new Error(
      'Native prices must not use the partial legacy options view'
    )
  },
  SettingsPageFrame: ({ children }: { children: ReactNode }) => children,
}))
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider, notifyManager } =
  await import('@tanstack/react-query')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { NativePricingSection } = await import('./native-pricing-section')
notifyManager.setScheduler(queueMicrotask)
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
const previous = useAuthStore.getState().auth
const adapter = api.defaults.adapter
let dispose: (() => Promise<void>) | undefined
afterEach(async () => {
  await dispose?.()
  dispose = undefined
  api.defaults.adapter = adapter
  useAuthStore.setState({ auth: previous })
  document.body.replaceChildren()
})
after(() => dom.close())

function configuration() {
  const config = {
    version: { routing: 3, pricing: 2 },
    secret_storage_enabled: false,
    routing: {
      version: 3,
      upstreams: [
        {
          id: 3,
          endpoint: 'https://supplier.example/v1',
          credential: { id: 9, revision: 1 },
          protocol: 'deepseek_search',
          enabled: true,
        },
      ],
      groups: [{ name: 'default', enabled: true }],
      aliases: [],
      routes: [
        {
          id: 7,
          model: 'deepseek-search',
          group: 'default',
          enabled: true,
          speeds: ['standard'],
          targets: [
            {
              id: 5,
              upstream_id: 3,
              upstream_model: 'deepseek-search',
              enabled: true,
              priority: 0,
              weight: 1,
              speeds: ['standard'],
              payload: 'exact',
              timeout_ms: 30000,
            },
          ],
          failover: {
            max_attempts: 1,
            retry_on: [],
            allow_replay_after_send: false,
          },
        },
      ],
    },
    prices: {
      version: 2,
      rules: [
        {
          id: 19,
          model: 'imported-model',
          group: 'default',
          speed: 'standard',
          rates: {
            input: 2000000,
            output: 6000000,
            cache_read: 500000,
            cache_write: 0,
            request: 0,
          },
          group_multiplier_ppm: 750000,
          speed_multiplier_ppm: 1000000,
        },
      ],
    },
    bindings: {
      names: {},
      contexts: [],
      vendors: {},
      models: {},
      discoveries: {},
      price_drafts: {},
    },
  }
  const source = config.routing.routes[0]
  config.routing.upstreams.push({
    ...config.routing.upstreams[0],
    id: 9,
    protocol: 'openai_chat',
    endpoint: 'http://127.0.0.1:17112',
  })
  for (const [index, model] of [
    'imported-model',
    'imported-model-copy',
    'openrouter/auto-beta',
  ].entries()) {
    config.routing.routes.push({
      ...source,
      id: 20 + index,
      model,
      targets: [
        {
          ...source.targets[0],
          id: 50 + index,
          upstream_id: 9,
          upstream_model: model,
        },
      ],
    })
  }
  return config
}
async function mount(
  nativeConfiguration = true,
  role = 100,
  rejectPublication = false,
  prepare?: (current: ReturnType<typeof configuration>) => void
) {
  useAuthStore.getState().auth.setBundle({
    access_token: 'synthetic-pricing-session',
    token_type: 'Bearer',
    access_expires_at: 2_000_000_000,
    user: { id: 501, role, username: 'synthetic-admin' },
    session: {
      sid: 'synthetic-pricing-sid',
      current: true,
      login_method: 'password',
      ip: '127.0.0.1',
      user_agent: 'synthetic',
      created_at: 1,
      last_active_at: 1,
      expires_at: 2_000_000_000,
    },
  })
  const requests: string[] = []
  const writes: Array<{ path: string; body: Record<string, unknown> }> = []
  let current = configuration()
  prepare?.(current)
  api.defaults.adapter = async (config) => {
    requests.push(config.url ?? '')
    if (config.method !== 'get') {
      const body =
        typeof config.data === 'string' ? JSON.parse(config.data) : config.data
      writes.push({ path: config.url ?? '', body })
      assert.deepEqual(body.expected_version, { routing: 3, pricing: 2 })
      let data: unknown
      if (config.url === '/core/v1/admin/routing/prices/preview') {
        assert.equal(config.method, 'post')
        data = {
          confirmation_digest: 'a'.repeat(64),
          before: current.prices,
          after: body.prices,
        }
      } else if (config.url === '/core/v1/admin/routing/prices') {
        assert.equal(config.method, 'put')
        assert.equal(body.confirmation_digest, 'a'.repeat(64))
        if (rejectPublication) throw new Error('Synthetic version conflict')
        current = {
          ...current,
          version: { routing: 4, pricing: 3 },
          routing: { ...current.routing, version: 4 },
          prices: body.prices,
        }
        data = { version: current.version }
      } else assert.fail(`Unexpected write: ${config.url}`)
      return {
        config,
        headers: {},
        status: 200,
        statusText: 'OK',
        data: { success: true, data },
      }
    }
    const data =
      config.url === '/api/channel/ops'
        ? { native_configuration: nativeConfiguration, native_secrets: false }
        : config.url === '/core/v1/admin/routing/configuration'
          ? current
          : config.url === '/core/v1/admin/routing/pricing-source-snapshot'
            ? {
                contract: 'core.legacy-pricing-snapshot.v1',
                purpose: 'reference_only',
                source_sha256: 'b'.repeat(64),
                observed_at: 1700000000,
                options: [
                  {
                    key: 'UserUsableGroups',
                    value: '{"default":"Original description"}',
                  },
                ],
                enabled_channels: [],
                unsupported: [],
              }
            : assert.fail(`Unexpected request: ${config.url}`)
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: { success: true, data },
    }
  }
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  await act(async () =>
    root.render(
      <QueryClientProvider client={client}>
        <I18nextProvider i18n={i18n}>
          <NativePricingSection
            groupWorkspace={section === 'group-pricing'}
            initialMode={
              section === 'router-pricing' ? 'upstream_cost' : undefined
            }
          />
        </I18nextProvider>
      </QueryClientProvider>
    )
  )
  dispose = async () => {
    await act(async () => root.unmount())
    client.clear()
  }
  return { container, requests, writes }
}
async function until(check: () => boolean) {
  for (let attempt = 0; attempt < 100 && !check(); attempt++) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 10))
    })
  }
  assert.ok(check(), 'Native pricing did not reach the expected state')
}
function modelRows(container: ParentNode) {
  return Array.from(container.querySelector('table > tbody')?.children ?? [])
}

function meterRates(row: Element, label: string) {
  const meters = row.querySelector('details table')
  assert.ok(meters)
  const meter = [...meters.querySelectorAll('tbody tr')].find(
    (entry) => entry.querySelector('th')?.textContent === label
  )
  assert.ok(meter, `Missing price meter: ${label}`)
  return [...meter.querySelectorAll('td')].map((cell) => cell.textContent)
}

for (const entry of ['model-pricing']) {
  test(`${entry} reads published prices instead of partial legacy settings`, async () => {
    section = entry
    const { container, requests } = await mount()
    await until(() =>
      Boolean(
        container
          .querySelector('table')
          ?.textContent?.includes('imported-model')
      )
    )
    const table = container.querySelector('table')
    assert.ok(table)
    assert.match(table.textContent ?? '', /0\.75 × 1/)
    const [model] = modelRows(container)
    assert.ok(model)
    assert.deepEqual(meterRates(model, 'Input'), ['2', '1.5'])
    assert.deepEqual(meterRates(model, 'Output'), ['6', '4.5'])
    assert.deepEqual(meterRates(model, 'Cache read'), ['0.5', '0.375'])
    assert.equal(container.querySelector('pre'), null)
    assert.doesNotMatch(container.textContent ?? '', /Original configuration/)
    assert.ok(
      !requests.includes('/core/v1/admin/routing/pricing-source-snapshot')
    )
    assert.ok(requests.includes('/core/v1/admin/routing/configuration'))
    assert.ok(!requests.some((url) => url.startsWith('/api/option')))
  })
}
test('ordinary pricing renders the effective tier columns and opens the model’s official editor', async () => {
  section = 'model-pricing'
  const { container, writes } = await mount(true, 100, false, (config) => {
    Object.assign(config.prices.rules[0], {
      billing_mode: 'tiered_decimal',
      rates: { input: 0, output: 0, cache_read: 0, cache_write: 0, request: 0 },
      tiered: {
        tiers: [
          {
            up_to_input_tokens: null,
            rates: {
              input: { numerator: 1, denominator: 2000000 },
              output: { numerator: 3, denominator: 2 },
              cache_read: { numerator: 0, denominator: 1 },
            },
          },
        ],
      },
    })
  })
  await until(() => !!container.querySelector('tbody tr'))
  const row = container.querySelector('tbody tr')
  assert.ok(row)
  assert.deepEqual(meterRates(row, 'Input'), ['0.0000005', '0.000000375'])
  assert.deepEqual(meterRates(row, 'Output'), ['1.5', '1.125'])
  assert.deepEqual(meterRates(row, 'Cache read'), ['0', '0'])
  assert.deepEqual(meterRates(row, 'Cache write'), ['Unknown', 'Unknown'])
  assert.deepEqual(meterRates(row, 'Request fee · USD/Request'), [
    'No separate rate',
  ])
  await clickPriceAction('Official base price', row)
  const editor = () =>
    document.querySelector('section[aria-label="Official base price"]')
  await until(() => Boolean(editor()?.querySelector('textarea')))
  assert.equal(editor()?.querySelector('h3')?.textContent, 'imported-model')
  assert.equal(writes.length, 0)
})

test('group pricing shows one group row with old description and editable multiplier', async () => {
  section = 'group-pricing'
  const { container, requests, writes } = await mount()
  await until(
    () => container.textContent?.includes('Original description') === true
  )
  const row = container.querySelector('tbody tr')
  assert.ok(row)
  assert.match(row.textContent ?? '', /default.*Original description.*0\.75×/)
  assert.doesNotMatch(
    container.textContent ?? '',
    /Original configuration|Official base price/
  )
  assert.doesNotMatch(
    container.querySelector('table')?.textContent ?? '',
    /imported-model/
  )
  assert.equal(container.querySelector('[role="tab"]'), null)
  await clickButton('Edit')
  const field = container.querySelector<HTMLInputElement>(
    'input[inputmode="decimal"]'
  )
  assert.ok(field)
  assert.equal(field.value, '0.75')
  const setValue = Object.getOwnPropertyDescriptor(
    dom.HTMLInputElement.prototype,
    'value'
  )?.set
  assert.ok(setValue)
  await act(async () => {
    setValue.call(field, '1.25')
    field.dispatchEvent(new Event('input', { bubbles: true }))
  })
  await clickButton('Apply to group')
  assert.ok(requests.includes('/core/v1/admin/routing/pricing-source-snapshot'))
  assert.equal(writes.length, 0)
  await clickButton('Preview changes')
  await until(() => writes.length === 1)
  const after = writes[0].body.prices as ReturnType<
    typeof configuration
  >['prices']
  assert.equal(after.rules[0].group_multiplier_ppm, 1250000)
})

test('router pricing stages only eligible upstream-cost rules and preserves ordinary prices through publication', async () => {
  section = 'router-pricing'
  const { container, requests, writes } = await mount()
  await until(
    () => container.textContent?.includes('openrouter/auto-beta') === true
  )
  assert.doesNotMatch(
    container.textContent ?? '',
    /Original configuration|Official base price|imported-model|deepseek-search|JSON|Cache write/
  )
  assert.equal(container.querySelector('table'), null)
  assert.ok(
    !requests.includes('/core/v1/admin/routing/pricing-source-snapshot')
  )
  await clickButton('Add price rule')
  await clickButton('Apply to draft')
  await until(
    () => container.textContent?.includes('Pending price changes') === true
  )
  assert.equal(writes.length, 0)
  await clickButton('Preview changes')
  await until(() => writes.length === 1)
  const after = writes[0].body.prices as { rules: Record<string, unknown>[] }
  assert.equal(after.rules.length, 2)
  assert.deepEqual(after.rules[0], configuration().prices.rules[0])
  assert.equal(after.rules[1].model, 'openrouter/auto-beta')
  assert.equal(after.rules[1].billing_mode, 'upstream_cost')
  assert.equal(after.rules[1].group_multiplier_ppm, 1_000_000)
  await clickButton('Confirm and publish')
  await until(
    () =>
      writes.length === 2 &&
      !container.textContent?.includes('Pending price changes')
  )
  assert.equal(writes[1].body.confirmation_digest, 'a'.repeat(64))
  assert.deepEqual(writes[1].body.prices, after)
  assert.doesNotMatch(
    container.textContent ?? '',
    /imported-model|deepseek-search/
  )
  await clickButton('Edit')
  assert.equal(
    container.querySelector<HTMLInputElement>('input[inputmode="decimal"]')
      ?.value,
    '1'
  )
  await clickButton('Cancel')
  await clickButton('Remove')
  await until(
    () => container.textContent?.includes('Pending price changes') === true
  )
  assert.equal(writes.length, 2)
})

test('router page excludes incompatible candidates and mixed drafts and keeps stored router rules visible', async () => {
  section = 'router-pricing'
  const { container, writes } = await mount(true, 100, false, (current) => {
    const source = current.routing.routes[0]
    // Even a recognized router model is ineligible on an external target.
    current.routing.routes.push({
      ...source,
      id: 30,
      model: 'nvidia/switchyard',
    })
    const ordinary = current.prices.rules[0]
    const stored = {
      ...ordinary,
      id: 28,
      model: 'openrouter/fusion',
      billing_mode: 'upstream_cost',
      rates: { input: 0, output: 0, cache_read: 0, cache_write: 0, request: 0 },
    }
    current.prices.rules.push(stored)
    Object.assign(current.bindings.price_drafts, {
      '41': {
        id: 41,
        name: 'Router-only draft',
        base_pricing_version: 2,
        updated_by: 501,
        updated_at: 1700000000,
        prices: {
          version: 3,
          rules: [ordinary, { ...stored, group_multiplier_ppm: 900000 }],
        },
      },
      '42': {
        id: 42,
        name: 'Mixed model draft',
        base_pricing_version: 2,
        updated_by: 501,
        updated_at: 1700000000,
        prices: {
          version: 3,
          rules: [{ ...ordinary, group_multiplier_ppm: 800000 }, stored],
        },
      },
    })
  })
  await until(
    () => container.textContent?.includes('openrouter/fusion') === true
  )
  assert.match(container.textContent ?? '', /Router-only draft/)
  assert.doesNotMatch(
    container.textContent ?? '',
    /Mixed model draft|nvidia\/switchyard|imported-model/
  )
  assert.match(container.textContent ?? '', /Capability unavailable/)
  assert.equal(writes.length, 0)
})

test('unavailable native configuration does not open a price editor', async () => {
  section = 'model-pricing'
  const { container, requests } = await mount(false)
  await until(
    () => container.textContent?.includes('Capability unavailable') === true
  )
  assert.ok(!requests.includes('/core/v1/admin/routing/configuration'))
  assert.equal(container.querySelector('textarea'), null)
})

async function clickButton(label: string, within: ParentNode = document) {
  const button = [...within.querySelectorAll<HTMLButtonElement>('button')].find(
    (element) => element.textContent === label
  )
  assert.ok(button, `Missing button: ${label}`)
  await act(async () => button.click())
}
async function clickPriceAction(label: string, row?: Element) {
  await clickButton('Actions', row ?? document)
  await until(() =>
    [...document.querySelectorAll<HTMLElement>('[role="menuitem"]')].some(
      (element) => element.textContent === label
    )
  )
  const item = [
    ...document.querySelectorAll<HTMLElement>('[role="menuitem"]'),
  ].find((element) => element.textContent === label)
  assert.ok(item, `Missing price action: ${label}`)
  await act(async () => item.click())
}

async function editInputPrice(value: string) {
  await clickPriceAction('Edit')
  await until(() => document.querySelector('#native-price-rule-form') !== null)
  const label = [...document.querySelectorAll('label')].find((element) =>
    element.textContent?.includes('Input price (USD / 1M tokens)')
  )
  const input = label?.querySelector('input')
  assert.ok(input)
  const setValue = Object.getOwnPropertyDescriptor(
    dom.HTMLInputElement.prototype,
    'value'
  )?.set
  assert.ok(setValue)
  await act(async () => {
    setValue.call(input, value)
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
  const form = document.querySelector('form#native-price-rule-form')
  assert.ok(form)
  await act(async () =>
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
  )
}

test('row actions copy exact prices and remove only the chosen draft rule', async () => {
  section = 'model-pricing'
  const { container, writes } = await mount()
  await until(() => container.querySelector('tbody tr') !== null)
  await clickPriceAction('Copy')
  await until(() => document.querySelector('#native-price-rule-form') !== null)
  const form = document.querySelector<HTMLFormElement>(
    '#native-price-rule-form'
  )
  assert.ok(form)
  const model = form.querySelector<HTMLInputElement>(
    'input[list="native-price-models"]'
  )
  assert.equal(model?.value, 'imported-model-copy')
  await act(async () =>
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
  )
  await until(() => modelRows(container).length === 2)
  const copied = modelRows(container).find((row) =>
    row.textContent?.includes('imported-model-copy')
  )
  assert.ok(copied)
  assert.deepEqual(meterRates(copied, 'Input'), ['2', '1.5'])
  assert.deepEqual(meterRates(copied, 'Output'), ['6', '4.5'])
  assert.deepEqual(meterRates(copied, 'Cache read'), ['0.5', '0.375'])
  await clickPriceAction('Remove', copied)
  await until(() => modelRows(container).length === 1)
  assert.doesNotMatch(
    container.querySelector('tbody')?.textContent ?? '',
    /imported-model-copy/
  )
  assert.match(
    container.querySelector('tbody')?.textContent ?? '',
    /imported-model/
  )
  assert.equal(writes.length, 0)
})

test('visual edits publish exact rates only after preview and confirmation', async () => {
  section = 'model-pricing'
  const { container, writes } = await mount()
  await until(
    () =>
      container
        .querySelector('table')
        ?.textContent?.includes('imported-model') === true
  )
  await editInputPrice('2.000001')
  assert.equal(writes.length, 0)
  await until(
    () => container.textContent?.includes('Pending price changes') === true
  )
  await clickButton('Preview changes')
  await until(
    () => document.body.textContent?.includes('Confirm and publish') === true
  )
  assert.equal(writes.length, 1)
  assert.equal(
    (writes[0].body.prices as ReturnType<typeof configuration>['prices'])
      .rules[0].rates.input,
    2_000_001
  )
  await clickButton('Confirm and publish')
  await until(
    () =>
      writes.length === 2 &&
      !container.textContent?.includes('Pending price changes')
  )
  assert.equal(writes[1].body.confirmation_digest, 'a'.repeat(64))
  assert.deepEqual(writes[1].body.prices, writes[0].body.prices)
})

test('a failed publication keeps the edited prices for recovery', async () => {
  section = 'model-pricing'
  const { container, writes } = await mount(true, 100, true)
  await until(
    () =>
      container
        .querySelector('table')
        ?.textContent?.includes('imported-model') === true
  )
  await editInputPrice('4.5')
  await clickButton('Preview changes')
  await until(
    () => document.body.textContent?.includes('Confirm and publish') === true
  )
  await clickButton('Confirm and publish')
  await until(
    () =>
      document.body.textContent?.includes('Synthetic version conflict') === true
  )
  assert.equal(writes.length, 2)
  assert.match(container.textContent ?? '', /Pending price changes/)
  assert.match(container.querySelector('table')?.textContent ?? '', /4\.5/)
})
test('an ordinary session cannot read native administrative prices', async () => {
  section = 'model-pricing'
  const { container, requests } = await mount(true, 1)
  await until(
    () =>
      container.textContent?.includes('Administrator access required') === true
  )
  assert.ok(!requests.includes('/core/v1/admin/routing/configuration'))
})

test('a missing routed tool price is drafted in exact request units and requires confirmation', async () => {
  section = 'model-pricing'
  const { container, requests, writes } = await mount()
  await until(() => container.querySelector('[role="tab"]') !== null)
  await clickButton('Unset price models')
  await until(
    () =>
      container
        .querySelector('[role="tabpanel"]:not([hidden])')
        ?.textContent?.includes('deepseek-search') === true
  )
  await clickButton('Tool prices')
  await clickButton('Add price rule')
  const field = [...container.querySelectorAll('label')]
    .find((label) => label.textContent?.includes('Request fee (USD / request)'))
    ?.querySelector('input')
  assert.ok(field)
  const setValue = Object.getOwnPropertyDescriptor(
    dom.HTMLInputElement.prototype,
    'value'
  )?.set
  assert.ok(setValue)
  await act(async () => {
    setValue.call(field, '0.003001')
    field.dispatchEvent(new Event('input', { bubbles: true }))
  })
  await act(async () =>
    field.form?.dispatchEvent(
      new Event('submit', { bubbles: true, cancelable: true })
    )
  )
  assert.equal(writes.length, 0)
  await clickButton('Preview changes')
  await until(
    () => document.body.textContent?.includes('Confirm and publish') === true
  )
  const rules = (
    writes[0].body.prices as ReturnType<typeof configuration>['prices']
  ).rules
  assert.deepEqual(
    rules.find((rule) => rule.model === 'deepseek-search')?.rates,
    {
      input: 0,
      output: 0,
      cache_read: 0,
      cache_write: 0,
      request: 3001,
    }
  )
  assert.deepEqual(rules[0], configuration().prices.rules[0])
  await clickButton('Confirm and publish')
  await until(
    () =>
      writes.length === 2 &&
      !container.textContent?.includes('Pending price changes')
  )
  assert.deepEqual(writes[1].body.prices, writes[0].body.prices)
  assert.equal(
    requests.some((path) => path.includes('ratio_sync')),
    false
  )
})
