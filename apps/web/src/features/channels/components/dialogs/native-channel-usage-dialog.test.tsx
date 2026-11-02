/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { Window } from 'happy-dom'

import {
  readNativeRoutingScope,
  type NativeConfiguration,
} from '../../native-api'

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
  writable: true,
  value: true,
})
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { NativeLegacyBalanceSettings } =
  await import('./native-channel-usage-dialog')
const adapter = api.defaults.adapter
const request = api.request
const previous = useAuthStore.getState().auth
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
let dispose: (() => Promise<void>) | undefined
afterEach(async () => {
  await dispose?.()
  dispose = undefined
  api.request = request
  api.defaults.adapter = adapter
  useAuthStore.setState({ auth: previous })
  document.body.replaceChildren()
})
after(() => dom.close())
function login(sid = 'synthetic-routing-editor', role = 10) {
  useAuthStore.getState().auth.setBundle({
    access_token: 'synthetic-channel-session-token',
    token_type: 'Bearer',
    access_expires_at: 2_000_000_000,
    user: { id: 501, role, username: 'synthetic-admin' },
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
function fixture(): NativeConfiguration {
  const names: NativeConfiguration['bindings']['names'] = {
    1: 'Private provider',
  }
  return {
    version: { routing: 1, pricing: 1 },
    secret_storage_enabled: true,
    routing: {
      version: 1,
      upstreams: [
        {
          id: 1,
          endpoint: 'https://private-provider.example/v1/',
          protocol: 'openai_chat',
          credential: { id: 7, revision: 1 },
          enabled: true,
        },
      ],
      groups: [{ name: 'default', enabled: true }],
      aliases: [],
      routes: [
        {
          id: 1,
          model: 'model-a',
          group: 'default',
          enabled: true,
          speeds: ['standard'],
          targets: [
            {
              id: 1,
              upstream_id: 1,
              upstream_model: 'model-a',
              enabled: true,
              priority: 0,
              weight: 1,
              speeds: ['standard'],
              payload: 'adapt',
              timeout_ms: 30_000,
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
      version: 1,
      rules: [
        {
          id: 1,
          model: 'model-a',
          group: 'default',
          speed: 'standard',
          rates: {
            input: 2,
            output: 4,
            cache_read: 0,
            cache_write: 0,
            request: 0,
          },
          group_multiplier_ppm: 1_000_000,
          speed_multiplier_ppm: 1_000_000,
        },
      ],
    },
    bindings: {
      names,
      contexts: [],
      vendors: {},
      models: {},
      discoveries: {},
      price_drafts: {},
    },
  }
}

async function mount() {
  login()
  const scope = readNativeRoutingScope()
  assert.ok(scope)
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  await act(async () =>
    root.render(
      <I18nextProvider i18n={i18n}>
        <NativeLegacyBalanceSettings scope={scope} channelId={1} />
      </I18nextProvider>
    )
  )
  dispose = async () => {
    await act(async () => root.unmount())
  }
  return container
}
async function click(container: Element, label: string) {
  const button = [...container.querySelectorAll('button')].find(
    (element) => element.textContent === label
  )
  assert.ok(button, label)
  await act(async () => {
    button.click()
  })
  return button
}
async function choose(container: Element, kind: string) {
  const select = container.querySelector('select')
  assert.ok(select)
  await act(async () => {
    select.value = kind
    select.dispatchEvent(new Event('change', { bubbles: true }))
  })
}
async function setRate(container: Element, rate: string) {
  const input = container.querySelector('input')
  assert.ok(input)
  const setter = Object.getOwnPropertyDescriptor(
    dom.HTMLInputElement.prototype,
    'value'
  )?.set
  assert.ok(setter)
  await act(async () => {
    setter.call(input, rate)
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
}
async function submit(container: Element) {
  const form = container.querySelector('form')
  assert.ok(form)
  await act(async () => {
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
  })
}

test('automatic balance editor reads manual state, saves only on submit, reloads rate and explicitly disables', async () => {
  const config = fixture()
  config.bindings.balance_probes = {
    '1': { path: '/manual', json_pointer: '/points', unit_label: 'points' },
  }
  config.bindings.legacy_balance = { '1': { channel_type: 43 } }
  const writes: unknown[] = []
  let reads = 0
  api.defaults.adapter = async (requestConfig) => {
    let data: unknown
    if (requestConfig.method === 'get') {
      reads++
      assert.equal(requestConfig.url, '/core/v1/admin/routing/configuration')
      data = config
    } else {
      assert.equal(
        requestConfig.url,
        '/core/v1/admin/routing/channels/1/balance-config'
      )
      const input = JSON.parse(String(requestConfig.data))
      writes.push(input)
      assert.deepEqual(input.expected_version, config.version)
      assert.equal(input.probe, null)
      config.version = {
        routing: config.version.routing + 1,
        pricing: config.version.pricing,
      }
      config.routing.version = config.version.routing
      config.bindings.balance_probes = {}
      config.bindings.legacy_balance = input.legacy ? { '1': input.legacy } : {}
      data = { version: config.version, probe: null, legacy: input.legacy }
    }
    return {
      config: requestConfig,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: { success: true, data },
    }
  }
  const container = await mount()
  assert.equal(reads, 0)
  await click(container, 'channelBalance.configure · Automatic')
  assert.match(container.textContent ?? '', /Manual:.*\/manual.*→ Automatic/)
  assert.equal(container.querySelector('select')?.value, '43')
  await choose(container, '25')
  await setRate(container, '7.2500')
  assert.equal(writes.length, 0, 'editing must never publish')
  await submit(container)
  assert.deepEqual(writes[0], {
    expected_version: { routing: 1, pricing: 1 },
    probe: null,
    legacy: { channel_type: 25, cny_per_usd: '7.2500' },
  })
  await click(container, 'Refresh')
  assert.equal(container.querySelector('select')?.value, '25')
  assert.equal(container.querySelector('input')?.value, '7.2500')
  assert.ok(!container.textContent?.includes('/manual'))
  await choose(container, '12')
  assert.ok(container.textContent?.includes('channelBalance.unitUnknown'))
  assert.equal(container.querySelector('input'), null)
  await submit(container)
  assert.deepEqual(writes[1], {
    expected_version: { routing: 2, pricing: 1 },
    probe: null,
    legacy: { channel_type: 12 },
  })
  await click(container, 'Disable')
  assert.deepEqual(writes[2], {
    expected_version: { routing: 3, pricing: 1 },
    probe: null,
    legacy: null,
  })
  assert.equal(container.querySelector('select')?.value, '')
  assert.equal(writes.length, 3)
})

test('unknown save result freezes the draft until explicit reload; invalid rates never dispatch', async () => {
  const config = fixture()
  config.bindings.legacy_balance = { '1': { channel_type: 25 } }
  let writes = 0
  api.defaults.adapter = async (requestConfig) => {
    if (requestConfig.method !== 'get') writes++
    return {
      config: requestConfig,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: {
        success: true,
        data:
          requestConfig.method === 'get'
            ? config
            : { version: { routing: 2, pricing: 1 } },
      },
    }
  }
  const container = await mount()
  await click(container, 'channelBalance.configure · Automatic')
  await setRate(container, '0')
  await submit(container)
  assert.equal(writes, 0)
  assert.ok(
    container.textContent?.includes('Exchange rate must be greater than 0')
  )
  await setRate(container, '7')
  await submit(container)
  assert.equal(writes, 1)
  assert.ok(container.textContent?.includes('channelBalance.saveError'))
  assert.equal(container.querySelector('select')?.disabled, true)
  assert.equal(container.querySelector('input')?.value, '7')
  await submit(container)
  assert.equal(writes, 1, 'unknown outcomes cannot be replayed')
  await click(container, 'Refresh')
  assert.equal(container.querySelector('select')?.disabled, false)
  assert.equal(container.querySelector('input')?.value, '')
})
