/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { Window } from 'happy-dom'

import { parseNativeDocument, type NativeConfiguration } from '../native-api'

type SuccessfulResponse<Data> = { data: { success: true; data: Data } }

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
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { NativeRoutingManagement } = await import('./native-routing-management')
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
function fixture() {
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
const accountingCapabilities = {
  native_channel_usage: true,
  native_channel_counters: true,
  native_balance_probe: true,
  copy_preserve_counters: true,
}
async function mount(
  initialSection: 'channels' | 'prices',
  nativeSecrets = true,
  accounting: Partial<typeof accountingCapabilities> = accountingCapabilities,
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
) {
  api.defaults.adapter = async (config) => {
    assert.equal(config.url, '/api/channel/ops')
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: {
        success: true,
        data: { native_secrets: nativeSecrets, ...accounting },
      },
    }
  }
  login()
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  await act(async () =>
    root.render(
      <QueryClientProvider client={client}>
        <I18nextProvider i18n={i18n}>
          <NativeRoutingManagement initialSection={initialSection} />
        </I18nextProvider>
      </QueryClientProvider>
    )
  )
  dispose = async () => {
    await act(async () => root.unmount())
    client.clear()
  }
  return container
}
async function until(check: () => boolean) {
  for (let attempt = 0; attempt < 100; attempt++) {
    if (check()) return
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 10))
    })
  }
  assert.ok(
    check(),
    'The native management view did not reach the expected state'
  )
}
function button(container: Element, text: string) {
  const value = [...container.querySelectorAll('button')].find(
    (element) => element.textContent === text
  )
  assert.ok(value, `Missing button ${text}`)
  return value as HTMLButtonElement
}

test('prices require a reviewed preview and editing invalidates the exact confirmation before one CAS publication', async () => {
  let config = { ...fixture(), channel_id_high_water: 17 }
  const writes: Array<{ path: string; body: Record<string, unknown> }> = []
  let previews = 0
  api.request = (async (options: {
    url: string
    method: string
    data?: Record<string, unknown>
  }) => {
    if (options.method === 'GET') {
      return { data: { success: true, data: config } }
    }
    assert.ok(options.data)
    assert.equal(Object.hasOwn(options.data, 'channel_id_high_water'), false)
    if (options.url.endsWith('/preview')) {
      previews++
      return {
        data: {
          success: true,
          data: {
            saved: false,
            after: options.data.prices,
            confirmation_digest: String(previews).repeat(64),
          },
        },
      }
    }
    writes.push({ path: options.url, body: options.data })
    config = {
      ...config,
      version: { routing: 2, pricing: 2 },
      routing: { ...config.routing, version: 2 },
      prices: options.data.prices as typeof config.prices,
    }
    return { data: { success: true, data: { version: config.version } } }
  }) as typeof api.request
  const container = await mount('prices')
  await until(() =>
    [...container.querySelectorAll('button')].some(
      (element) => element.textContent === 'Edit'
    )
  )
  await act(async () => button(container, 'Edit').click())
  assert.equal(container.textContent?.includes('Confirm and apply'), false)
  await act(async () => button(container, 'Preview').click())
  await until(() => !!container.textContent?.includes('Confirm and apply'))
  assert.equal(writes.length, 0)
  const textarea = container.querySelector('textarea')
  assert.ok(textarea)
  const data = JSON.parse(textarea.value)
  data.prices.rules[0].rates.input = 10
  const setter = Object.getOwnPropertyDescriptor(
    dom.HTMLTextAreaElement.prototype,
    'value'
  )?.set
  assert.ok(setter)
  await act(async () => {
    setter.call(textarea, JSON.stringify(data))
    textarea.dispatchEvent(new Event('input', { bubbles: true }))
  })
  assert.equal(container.textContent?.includes('Confirm and apply'), false)
  await act(async () => button(container, 'Preview').click())
  await until(() => !!container.textContent?.includes('Confirm and apply'))
  await act(async () => button(container, 'Confirm and apply').click())
  await until(() => writes.length === 1 && !container.querySelector('textarea'))
  assert.equal(previews, 2)
  assert.equal(writes[0].path, '/core/v1/admin/routing/prices')
  assert.deepEqual(writes[0].body.expected_version, { routing: 1, pricing: 1 })
  assert.equal(writes[0].body.confirmation_digest, '2'.repeat(64))
  assert.equal(
    (writes[0].body.prices as typeof config.prices).rules[0].rates.input,
    10
  )
})

test('private channel data and an unsaved secret disappear when the administrator session changes', async () => {
  let reads = 0
  api.request = (async () => {
    reads++
    return { data: { success: true, data: fixture() } }
  }) as typeof api.request
  const container = await mount('channels')
  await until(
    () => !!container.textContent?.includes('private-provider.example')
  )
  await act(async () => button(container, 'Create Channel').click())
  const secret = container.querySelector('input[type=password]')
  assert.ok(secret)
  await act(async () => login('ordinary-replacement', 1))
  await until(
    () => !!container.textContent?.includes('Administrator access required')
  )
  assert.equal(container.querySelector('textarea'), null)
  assert.equal(container.querySelector('input[type=password]'), null)
  assert.equal(
    container.textContent?.includes('private-provider.example'),
    false
  )
  assert.equal(reads, 1)
  await act(async () => login('another-admin', 10))
  await until(
    () =>
      reads === 2 &&
      !!container.textContent?.includes('private-provider.example')
  )
})

async function openCopy(container: Element) {
  await until(() =>
    [...container.querySelectorAll('button')].some(
      (item) => item.textContent === 'Copy Channel'
    )
  )
  await act(async () => button(container, 'Copy Channel').click())
  await until(() => !!document.querySelector('[role=dialog]'))
  const dialog = document.querySelector('[role=dialog]')
  assert.ok(dialog)
  return dialog
}
async function changeSuffix(dialog: Element, suffix: string) {
  const input = dialog.querySelector('input')
  assert.ok(input)
  const setter = Object.getOwnPropertyDescriptor(
    dom.HTMLInputElement.prototype,
    'value'
  )?.set
  assert.ok(setter)
  await act(async () => {
    setter.call(input, suffix)
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
  return input
}

test('native copy previews the suffix without counters or secrets, blocks double dispatch and refreshes the published channel', async () => {
  let config = fixture()
  let reads = 0
  let writes = 0
  let complete: (() => void) | undefined
  api.request = (async (options: {
    method: string
    url: string
    data?: unknown
  }) => {
    if (options.method === 'GET') {
      reads++
      return { data: { success: true, data: config } }
    }
    writes++
    assert.equal(options.url, '/core/v1/admin/routing/channels/1/copy')
    assert.deepEqual(options.data, {
      expected_version: { routing: 1, pricing: 1 },
      suffix: '_copy',
    })
    await new Promise<void>((resolve) => {
      complete = resolve
    })
    config = {
      ...config,
      version: { routing: 2, pricing: 1 },
      routing: {
        ...config.routing,
        version: 2,
        upstreams: [
          ...config.routing.upstreams,
          { ...config.routing.upstreams[0], id: 2 },
        ],
      },
      bindings: {
        ...config.bindings,
        names: { ...config.bindings.names, 2: 'Private provider_copy' },
      },
    }
    return { data: { success: true, data: { id: 2, version: config.version } } }
  }) as typeof api.request
  const container = await mount('channels')
  const dialog = await openCopy(container)
  assert.equal(dialog.querySelector('input')?.value, '_copy')
  assert.ok(dialog.textContent?.includes('Private provider_copy'))
  assert.equal(
    dialog.querySelector('[role=checkbox]')?.getAttribute('aria-checked'),
    'true'
  )
  assert.equal(dialog.querySelector('input[type=password]'), null)
  await act(async () => {
    button(dialog, 'Copy Channel').click()
    button(dialog, 'Copy Channel').click()
  })
  assert.equal(writes, 1)
  assert.equal(button(dialog, 'Copying...').disabled, true)
  await act(async () => complete?.())
  await until(
    () =>
      reads >= 2 &&
      [...container.querySelectorAll('h3')].some(
        (item) => item.textContent === 'Private provider_copy'
      )
  )
  assert.equal(writes, 1)
})

test('native copy preserves suffixes on failure and resets the draft after an administrative session change', async () => {
  let writes = 0
  api.request = (async (options: { method: string; data?: unknown }) => {
    if (options.method === 'GET') {
      return { data: { success: true, data: fixture() } }
    }
    writes++
    assert.deepEqual(options.data, {
      expected_version: { routing: 1, pricing: 1 },
      suffix: writes === 1 ? '_副本' : '',
    })
    return { data: { success: false, message: 'Copy denied', data: {} } }
  }) as typeof api.request
  const container = await mount('channels')
  const dialog = await openCopy(container)
  const input = await changeSuffix(dialog, '_副本')
  await act(async () => button(dialog, 'Copy Channel').click())
  await until(() => !!dialog.textContent?.includes('Copy denied'))
  assert.equal(input.value, '_副本')
  await changeSuffix(dialog, '')
  await act(async () => button(dialog, 'Copy Channel').click())
  await until(() => writes === 2 && !button(dialog, 'Copy Channel').disabled)
  assert.equal(input.value, '')
  await act(async () => login('replacement-copy-admin'))
  await until(() => !document.querySelector('[role=dialog]'))
  const reopened = await openCopy(container)
  assert.equal(reopened.querySelector('input')?.value, '_copy')
  assert.equal(writes, 2)
})

test('native copy stays visible but disabled when native secret capability or configured key storage is unavailable', async () => {
  for (const storageEnabled of [true, false]) {
    const config = { ...fixture(), secret_storage_enabled: storageEnabled }
    let writes = 0
    api.request = (async (options: { method: string }) => {
      if (options.method !== 'GET') writes++
      return { data: { success: true, data: config } }
    }) as typeof api.request
    const container = await mount('channels', !storageEnabled)
    await until(() =>
      [...container.querySelectorAll('button')].some(
        (item) => item.textContent === 'Copy Channel'
      )
    )
    assert.equal(button(container, 'Copy Channel').disabled, true)
    assert.ok(
      container.textContent?.includes('API Keys: Capability unavailable')
    )
    await act(async () => button(container, 'Copy Channel').click())
    assert.equal(document.querySelector('[role=dialog]'), null)
    assert.equal(writes, 0)
    await dispose?.()
    dispose = undefined
    document.body.replaceChildren()
  }
})

test('native copy previews the generated name for an unnamed bootstrap channel', async () => {
  const config = fixture()
  api.request = (async () => ({
    data: {
      success: true,
      data: { ...config, bindings: { ...config.bindings, names: {} } },
    },
  })) as typeof api.request
  const container = await mount('channels')
  const dialog = await openCopy(container)
  assert.match(dialog.textContent ?? '', /Channel 1_copy/)
})

function required<T>(value: T | null | undefined): T {
  assert.ok(value)
  return value
}

const usageFixture = {
  scope: 'native_channel_window',
  start_timestamp: 1_700_000_000,
  end_timestamp: 1_700_000_000 + 31 * 86400,
  gross_settled_credits: '9007199254740993',
  refunded_credits: '2',
  net_used_credits: '9007199254740991',
  settled_requests: '9007199254740993',
  pending_potential_requests: '4',
  unknown_provenance_requests: '5',
  credits_per_usd: '500000',
  historical_coverage: false,
}

test('native channel card opens confirmed consumption with exact values and explicit coverage', async () => {
  const en = await import('@/i18n/locales/en.json')
  i18n.addResourceBundle('en', 'translation', en.default.translation)
  const reads: string[] = []
  api.request = (async (options: { url: string; method: string }) => {
    reads.push(options.url)
    assert.equal(options.method, 'GET')
    return {
      data: {
        success: true,
        data: options.url.endsWith('/usage') ? usageFixture : fixture(),
      },
    }
  }) as typeof api.request
  const container = await mount('channels', false)
  await until(() => !!container.textContent?.includes('Private provider'))
  assert.equal(button(container, 'Consumption').disabled, false)
  await act(async () => button(container, 'Consumption').click())
  await until(() => !!document.querySelector('dl'))
  const dialog = required(document.querySelector('[role=dialog]'))
  const text = dialog.textContent ?? ''
  assert.ok(text.includes('Confirmed native consumption'))
  assert.ok(
    text.includes(
      'Historical consumption and provider balances are not included.'
    )
  )
  assert.ok(text.includes('9,007,199,254,740,993'))
  assert.ok(text.includes('Unsettled potential requests'))
  assert.ok(text.includes('Requests without attribution evidence'))
  assert.equal(reads.filter((path) => path.endsWith('/usage')).length, 1)
  await act(async () => button(dialog, 'Close').click())
  await until(() => !document.querySelector('[role=dialog]'))
})

test('native usage errors and reused channel identities never turn into zero consumption', async () => {
  const { AxiosError, AxiosHeaders } = await import('axios')
  for (const failure of [403, 409, 501, 'malformed'] as const) {
    api.request = (async (options: { url: string }) => {
      if (!options.url.endsWith('/usage')) {
        return { data: { success: true, data: fixture() } }
      }
      if (failure === 'malformed') {
        return {
          data: {
            success: true,
            data: { ...usageFixture, net_used_credits: null },
          },
        }
      }
      throw new AxiosError(
        'internal implementation detail',
        'ERR_BAD_RESPONSE',
        undefined,
        undefined,
        {
          status: failure,
          statusText: 'unavailable',
          headers: {},
          config: { headers: new AxiosHeaders() },
          data: {},
        }
      )
    }) as typeof api.request
    const container = await mount('channels')
    await until(() => !!container.textContent?.includes('Private provider'))
    await act(async () => button(container, 'Consumption').click())
    await until(() => !!document.querySelector('[role=dialog] [role=alert]'))
    const dialog = required(document.querySelector('[role=dialog]'))
    const text = dialog.textContent ?? ''
    assert.equal(dialog.querySelector('dl'), null)
    assert.equal(text.includes('internal implementation detail'), false)
    if (failure === 409) {
      assert.match(text, /ID was reused.*historical identity/)
    }
    if (failure === 403) assert.match(text, /do not have permission/)
    await act(async () => button(dialog, 'Close').click())
    await dispose?.()
    dispose = undefined
  }
})

test('closing usage or switching sessions aborts the read and discards delayed results', async () => {
  let finish: (() => void) | undefined
  const signals: Array<AbortSignal | undefined> = []
  api.request = (async (options: { url: string; signal?: AbortSignal }) => {
    if (!options.url.endsWith('/usage')) {
      return { data: { success: true, data: fixture() } }
    }
    signals.push(options.signal)
    return new Promise<SuccessfulResponse<typeof usageFixture>>((resolve) => {
      finish = () => resolve({ data: { success: true, data: usageFixture } })
    })
  }) as typeof api.request
  const container = await mount('channels')
  await until(() => !!container.textContent?.includes('Private provider'))
  await act(async () => button(container, 'Consumption').click())
  await until(() => signals.length === 1)
  await act(async () =>
    button(required(document.querySelector('[role=dialog]')), 'Close').click()
  )
  assert.equal(signals[0]?.aborted, true)
  await act(async () => finish?.())
  assert.equal(document.querySelector('dl'), null)
  await act(async () => button(container, 'Consumption').click())
  await until(() => signals.length === 2)
  await act(async () => {
    login('replacement-usage-session', 10)
    await new Promise((resolve) => setTimeout(resolve, 20))
  })
  assert.equal(signals[1]?.aborted, true)
  await act(async () => finish?.())
  assert.equal(document.querySelector('[role=dialog]'), null)
  assert.equal(container.textContent?.includes('9,007,199,254,740,993'), false)
})

test('a confirmed zero native window displays zero instead of an unavailable state', async () => {
  const zero = {
    ...usageFixture,
    gross_settled_credits: '0',
    refunded_credits: '0',
    net_used_credits: '0',
    settled_requests: '0',
    pending_potential_requests: '0',
    unknown_provenance_requests: '0',
  }
  api.request = (async (options: { url: string }) => ({
    data: {
      success: true,
      data: options.url.endsWith('/usage') ? zero : fixture(),
    },
  })) as typeof api.request
  const container = await mount('channels')
  await until(() => !!container.textContent?.includes('Private provider'))
  await act(async () => button(container, 'Consumption').click())
  await until(() => document.querySelectorAll('dd').length === 6)
  for (const value of document.querySelectorAll('dd')) {
    assert.match(value.textContent ?? '', /^0(?: Credits)?$/)
  }
  assert.equal(document.querySelector('[role=dialog] [role=alert]'), null)
})

const balanceFixture = {
  balance: '9007199254740993.123456789',
  unit_label: 'provider points',
  balance_updated_time: null,
  kind: 'copied',
  recorded_at: 1_700_000_000,
  source_channel_id: 9,
}

test('provider balance is separate from confirmed consumption and preserves unknown, copied and reset observations', async () => {
  for (const observation of [
    balanceFixture,
    {
      ...balanceFixture,
      balance: null,
      unit_label: null,
      kind: 'unknown',
      source_channel_id: null,
      recorded_at: null,
    },
    { ...balanceFixture, balance: '0', kind: 'reset' },
  ]) {
    api.request = (async (options: { url: string }) => ({
      data: {
        success: true,
        data: options.url.endsWith('/counters')
          ? { ...countersFixture, balance: observation }
          : fixture(),
      },
    })) as typeof api.request
    const container = await mount('channels')
    await until(() => !!container.textContent?.includes('Private provider'))
    await act(async () => button(container, 'Provider balance').click())
    await until(() => !!document.querySelector('[role=dialog] dl'))
    const dialog = required(document.querySelector('[role=dialog]'))
    const displayed = required(dialog.querySelector('.text-2xl')).textContent
    assert.equal(
      displayed,
      observation.balance === null
        ? '-'
        : `${observation.balance} provider points`
    )
    assert.match(dialog.textContent ?? '', /Last updated:\s*-/)
    assert.equal(
      dialog.textContent?.includes('Confirmed native consumption'),
      false
    )
    if (observation.kind === 'unknown') {
      assert.ok(dialog.textContent?.includes('Unit unknown'))
    }
    if (observation.kind === 'copied') {
      assert.ok(dialog.textContent?.includes('Copied observation'))
    }
    if (observation.kind === 'reset') {
      assert.ok(dialog.textContent?.includes('Reset during copy'))
    }
    await act(async () => button(dialog, 'Close').click())
    await dispose?.()
    dispose = undefined
  }
})

test('a missing probe is disabled and failed explicit refreshes retain unknown rather than inventing a number', async () => {
  const { AxiosError, AxiosHeaders } = await import('axios')
  for (const status of [501, 403]) {
    api.request = (async (options: { url: string }) => {
      if (options.url.endsWith('/counters')) {
        return { data: { success: true, data: countersFixture } }
      }
      if (!options.url.endsWith('/balance')) {
        return {
          data: {
            success: true,
            data: {
              ...fixture(),
              bindings: {
                ...fixture().bindings,
                balance_probes: {
                  1: {
                    path: '/balance',
                    json_pointer: '/balance',
                    unit_label: null,
                  },
                },
              },
            },
          },
        }
      }
      throw new AxiosError(
        'private internal details',
        'ERR_BAD_RESPONSE',
        undefined,
        undefined,
        {
          status,
          statusText: 'unavailable',
          headers: {},
          config: { headers: new AxiosHeaders() },
          data: {},
        }
      )
    }) as typeof api.request
    const container = await mount('channels')
    await until(() => !!container.textContent?.includes('Private provider'))
    await act(async () => button(container, 'Provider balance').click())
    await until(() => !!document.querySelector('[role=dialog] dl'))
    const dialog = required(document.querySelector('[role=dialog]'))
    await act(async () => button(dialog, 'Update Balance').click())
    await until(() => !!dialog.querySelector('[role=alert]'))
    assert.equal(dialog.querySelector('.text-2xl')?.textContent, '-')
    assert.equal(
      dialog.textContent?.includes('private internal details'),
      false
    )
    if (status === 501) {
      assert.match(dialog.textContent ?? '', /Balance queries are disabled/)
      assert.equal(button(dialog, 'Update Balance').disabled, true)
    }
    await act(async () => button(dialog, 'Close').click())
    await dispose?.()
    dispose = undefined
  }
})

async function changeInput(input: HTMLInputElement, value: string) {
  await act(async () => {
    required(
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set
    ).call(input, value)
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
}

test('balance configuration preloads published fields, rejects invalid paths and sends one CAS save or explicit disable', async () => {
  let config: NativeConfiguration = {
    ...fixture(),
    bindings: {
      ...fixture().bindings,
      balance_probes: {
        1: {
          path: '/account/balance',
          json_pointer: '/available',
          unit_label: 'points',
        },
      },
    },
  }
  const writes: unknown[] = []
  api.request = (async (options: {
    url: string
    method: string
    data?: unknown
  }) => {
    if (options.method === 'GET') {
      return { data: { success: true, data: config } }
    }
    writes.push(options.data)
    assert.equal(
      options.url,
      '/core/v1/admin/routing/channels/1/balance-config'
    )
    config = {
      ...config,
      version: { ...config.version, routing: config.version.routing + 1 },
      routing: { ...config.routing, version: config.routing.version + 1 },
    }
    return { data: { success: true, data: { version: config.version } } }
  }) as typeof api.request
  const container = await mount('channels')
  await until(() => !!container.textContent?.includes('Private provider'))
  await act(async () => button(container, 'Configure balance query').click())
  let dialog = required(document.querySelector('[role=dialog]'))
  const inputs = [...dialog.querySelectorAll('input')]
  assert.deepEqual(
    inputs.map((input) => input.value),
    ['/account/balance', '/available', 'points']
  )
  await changeInput(required(inputs[0]), '//elsewhere.test/balance')
  await act(async () => button(dialog, 'Save').click())
  assert.equal(writes.length, 0)
  assert.ok(dialog.querySelector('[role=alert]'))
  await changeInput(required(inputs[0]), '/balance')
  await changeInput(required(inputs[1]), '/a~1b')
  await changeInput(required(inputs[2]), '')
  await act(async () => {
    button(dialog, 'Save').click()
    button(dialog, 'Save').click()
  })
  await until(() => !document.querySelector('[role=dialog]'))
  assert.deepEqual(writes, [
    {
      expected_version: { routing: 1, pricing: 1 },
      probe: { path: '/balance', json_pointer: '/a~1b', unit_label: null },
    },
  ])
  await act(async () => button(container, 'Configure balance query').click())
  dialog = required(document.querySelector('[role=dialog]'))
  await act(async () => button(dialog, 'Disable').click())
  await until(() => !document.querySelector('[role=dialog]'))
  assert.deepEqual(writes[1], {
    expected_version: { routing: 2, pricing: 1 },
    probe: null,
  })
})

test('Native copy sends false only after explicitly preserving balance and display baselines', async () => {
  let copied: unknown
  api.request = (async (options: { method: string; data?: unknown }) => {
    if (options.method === 'GET') {
      return { data: { success: true, data: fixture() } }
    }
    copied = options.data
    return {
      data: {
        success: true,
        data: { id: 2, version: { routing: 2, pricing: 1 } },
      },
    }
  }) as typeof api.request
  const container = await mount('channels')
  const dialog = await openCopy(container)
  const checkbox = required(
    dialog.querySelector<HTMLElement>('[role=checkbox]')
  )
  await act(async () => checkbox.click())
  assert.equal(checkbox.getAttribute('aria-checked'), 'false')
  assert.ok(
    dialog.textContent?.includes('unknown historical coverage stays unknown')
  )
  await act(async () => button(dialog, 'Copy Channel').click())
  await until(() => !document.querySelector('[role=dialog]'))
  assert.deepEqual(copied, {
    expected_version: { routing: 1, pricing: 1 },
    suffix: '_copy',
    reset_balance: false,
  })
})

const countersFixture = {
  balance: {
    balance: null,
    unit_label: null,
    balance_updated_time: null,
    kind: 'unknown',
    recorded_at: null,
    source_channel_id: null,
  },
  native: {
    ...usageFixture,
    scope: 'native_channel_totals',
    start_timestamp: 0,
    gross_settled_credits: '20',
    refunded_credits: '2',
    net_used_credits: '18',
  },
  inherited: {
    gross_settled_credits: '5',
    refunded_credits: '1',
    net_used_credits: '4',
  },
  combined: {
    gross_settled_credits: '25',
    refunded_credits: '3',
    net_used_credits: '22',
  },
  historical_coverage: false,
  complete: false,
  used_quota: null,
  credits_per_usd: '500000',
}

test('display subtotals keep copied inheritance separate from native consumption and incomplete history', async () => {
  api.request = (async (options: { url: string }) => ({
    data: {
      success: true,
      data: options.url.endsWith('/usage')
        ? usageFixture
        : options.url.endsWith('/counters')
          ? countersFixture
          : fixture(),
    },
  })) as typeof api.request
  const container = await mount('channels')
  await until(() => !!container.textContent?.includes('Private provider'))
  await act(async () => button(container, 'Consumption').click())
  await until(
    () =>
      document.querySelectorAll('[aria-label="Display usage"] dd').length === 3
  )
  const section = required(
    document.querySelector('[aria-label="Display usage"]')
  )
  assert.deepEqual(
    [...section.querySelectorAll('dd')].map((value) => value.textContent),
    ['18 Credits', '4 Credits', '22 Credits']
  )
  assert.match(section.textContent ?? '', /not a complete cumulative total/)
  assert.match(section.textContent ?? '', /not new consumption/)
})

test('an absent inherited display baseline remains unknown rather than displaying zero', async () => {
  const absent = {
    ...countersFixture,
    inherited: null,
    combined: {
      gross_settled_credits: countersFixture.native.gross_settled_credits,
      refunded_credits: countersFixture.native.refunded_credits,
      net_used_credits: countersFixture.native.net_used_credits,
    },
  }
  api.request = (async (options: { url: string }) => ({
    data: {
      success: true,
      data: options.url.endsWith('/usage')
        ? usageFixture
        : options.url.endsWith('/counters')
          ? absent
          : fixture(),
    },
  })) as typeof api.request
  const container = await mount('channels')
  await until(() => !!container.textContent?.includes('Private provider'))
  await act(async () => button(container, 'Consumption').click())
  await until(
    () =>
      document.querySelectorAll('[aria-label="Display usage"] dd').length === 3
  )
  const values = [
    ...document.querySelectorAll('[aria-label="Display usage"] dd'),
  ].map((element) => element.textContent)
  assert.deepEqual(values, ['18 Credits', '-', '18 Credits'])
})

test('closing a stored balance read aborts it and session changes discard a delayed configuration save', async () => {
  let finishRead: (() => void) | undefined
  let finishSave: (() => void) | undefined
  let readSignal: AbortSignal | undefined
  let writes = 0
  api.request = (async (options: {
    url: string
    method: string
    signal?: AbortSignal
  }) => {
    if (options.url.endsWith('/counters')) {
      readSignal = options.signal
      return new Promise<SuccessfulResponse<typeof countersFixture>>(
        (resolve) => {
          finishRead = () =>
            resolve({ data: { success: true, data: countersFixture } })
        }
      )
    }
    if (options.method === 'PUT') {
      writes++
      return new Promise<
        SuccessfulResponse<{ version: NativeConfiguration['version'] }>
      >((resolve) => {
        finishSave = () =>
          resolve({
            data: {
              success: true,
              data: { version: { routing: 2, pricing: 1 } },
            },
          })
      })
    }
    return { data: { success: true, data: fixture() } }
  }) as typeof api.request
  const container = await mount('channels')
  await until(() => !!container.textContent?.includes('Private provider'))
  await act(async () => button(container, 'Provider balance').click())
  await until(() => !!readSignal)
  await act(async () =>
    button(required(document.querySelector('[role=dialog]')), 'Close').click()
  )
  assert.equal(readSignal?.aborted, true)
  await act(async () => finishRead?.())
  assert.equal(document.querySelector('[role=dialog]'), null)
  await act(async () => button(container, 'Configure balance query').click())
  const dialog = required(document.querySelector('[role=dialog]'))
  await act(async () => button(dialog, 'Disable').click())
  assert.equal(writes, 1)
  await act(async () => {
    login('replacement-balance-session', 10)
    await new Promise((resolve) => setTimeout(resolve, 20))
  })
  await act(async () => finishSave?.())
  assert.equal(document.querySelector('[role=dialog]'), null)
  assert.equal(
    container.textContent?.includes('The save result could not be confirmed'),
    false
  )
  assert.equal(writes, 1)
})

test('old managed Core keeps default copy compatible and blocks accounting controls and preservation', async () => {
  let writes = 0
  api.request = (async (options: {
    method: string
    url: string
    data?: unknown
  }) => {
    if (options.method === 'GET') {
      assert.equal(options.url, '/core/v1/admin/routing/configuration')
      return { data: { success: true, data: fixture() } }
    }
    writes++
    assert.equal(options.url, '/core/v1/admin/routing/channels/1/copy')
    assert.deepEqual(options.data, {
      expected_version: { routing: 1, pricing: 1 },
      suffix: '_copy',
    })
    return {
      data: {
        success: true,
        data: { id: 9, version: { routing: 2, pricing: 1 } },
      },
    }
  }) as typeof api.request
  const container = await mount('channels', true, {})
  await until(() => !!container.textContent?.includes('Private provider'))
  for (const key of [
    'channelUsage.open',
    'channelBalance.title',
    'channelBalance.configure',
  ]) {
    assert.equal(
      [...container.querySelectorAll('button')].some(
        (item) => item.textContent === i18n.t(key)
      ),
      false
    )
  }
  const dialog = await openCopy(container)
  const reset = required(dialog.querySelector('[role=checkbox]'))
  assert.ok(reset instanceof HTMLElement)
  assert.ok(reset.matches(':disabled, [aria-disabled="true"]'))
  await act(async () => reset.click())
  assert.equal(reset.getAttribute('aria-checked'), 'true')
  assert.equal(writes, 0)
  assert.equal(button(dialog, 'Copy Channel').disabled, false)
  await act(async () => button(dialog, 'Copy Channel').click())
  await until(() => !document.querySelector('[role=dialog]'))
  assert.equal(writes, 1)
})

test('accounting capabilities gate window reads, display counters and provider refresh independently', async () => {
  for (const windowEnabled of [true, false]) {
    const reads: string[] = []
    const config = fixture()
    api.request = (async (options: { url: string }) => {
      reads.push(options.url)
      return {
        data: {
          success: true,
          data: options.url.endsWith('/usage')
            ? usageFixture
            : options.url.endsWith('/counters')
              ? countersFixture
              : {
                  ...config,
                  bindings: {
                    ...config.bindings,
                    balance_probes: {
                      1: {
                        path: '/balance',
                        json_pointer: '/balance',
                        unit_label: null,
                      },
                    },
                  },
                },
        },
      }
    }) as typeof api.request
    const container = await mount('channels', true, {
      native_channel_usage: windowEnabled,
      native_channel_counters: !windowEnabled,
      native_balance_probe: false,
      copy_preserve_counters: false,
    })
    await until(() => !!container.textContent?.includes('Private provider'))
    const labels = new Set(
      [...container.querySelectorAll('button')].map((item) => item.textContent)
    )
    assert.equal(labels.has(i18n.t('channelUsage.open')), windowEnabled)
    assert.equal(labels.has(i18n.t('channelBalance.title')), !windowEnabled)
    assert.equal(labels.has(i18n.t('channelBalance.configure')), false)
    await act(async () =>
      button(
        container,
        i18n.t(windowEnabled ? 'channelUsage.open' : 'channelBalance.title')
      ).click()
    )
    await until(() => !!document.querySelector('dl'))
    const dialog = required(document.querySelector('[role=dialog]'))
    if (windowEnabled) {
      assert.equal(
        reads.some((url) => url.endsWith('/counters')),
        false
      )
      assert.equal(
        [...dialog.querySelectorAll('section')].some(
          (item) =>
            item.getAttribute('aria-label') === i18n.t('channelCounters.title')
        ),
        false
      )
    } else {
      assert.equal(
        reads.some((url) => url.endsWith('/usage')),
        false
      )
      assert.equal(button(dialog, 'Update Balance').disabled, true)
      await act(async () => button(dialog, 'Update Balance').click())
      assert.equal(
        reads.some((url) => url.endsWith('/balance')),
        false
      )
    }
    await dispose?.()
    dispose = undefined
  }
})

test('expired capabilities cannot submit a previously selected preserve copy or retain new API entry points', async () => {
  let writes = 0
  api.request = (async (options: { method: string }) => {
    if (options.method !== 'GET') writes++
    return { data: { success: true, data: fixture() } }
  }) as typeof api.request
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const container = await mount(
    'channels',
    true,
    accountingCapabilities,
    client
  )
  const dialog = await openCopy(container)
  const reset = required(dialog.querySelector('[role=checkbox]'))
  assert.ok(reset instanceof HTMLElement)
  await act(async () => reset.click())
  assert.equal(reset.getAttribute('aria-checked'), 'false')
  await act(async () => {
    client.setQueryData(
      ['channel-ops'],
      {
        success: true,
        data: { native_secrets: true, ...accountingCapabilities },
      },
      { updatedAt: Date.now() - 60_001 }
    )
  })
  await until(() => button(dialog, 'Copy Channel').disabled)
  assert.equal(button(dialog, 'Copy Channel').disabled, true)
  for (const key of [
    'channelUsage.open',
    'channelBalance.title',
    'channelBalance.configure',
  ]) {
    assert.equal(
      [...container.querySelectorAll('button')].some(
        (item) => item.textContent === i18n.t(key)
      ),
      false
    )
  }
  await act(async () => button(dialog, 'Copy Channel').click())
  assert.equal(writes, 0)
})

test('channel creation uses the permanent ID high water without publishing the read hint and keeps the old Core fallback', async () => {
  for (const highWater of [17, 0, undefined]) {
    const expectedId = highWater === 17 ? 18 : highWater === 0 ? 1 : 2
    const current = fixture()
    const config: NativeConfiguration = {
      ...current,
      ...(highWater === undefined ? {} : { channel_id_high_water: highWater }),
      routing: {
        ...current.routing,
        upstreams: highWater === 0 ? [] : current.routing.upstreams,
        routes: highWater === 0 ? [] : current.routing.routes,
      },
      bindings: {
        ...current.bindings,
        names: highWater === 0 ? {} : current.bindings.names,
      },
    }
    let writes = 0
    api.request = (async (options: {
      method: string
      url: string
      data?: unknown
    }) => {
      if (options.method === 'GET') {
        return { data: { success: true, data: config } }
      }
      writes++
      assert.equal(options.method, 'POST')
      assert.equal(options.url, '/core/v1/admin/routing/channels')
      assert.deepEqual(options.data, {
        expected_version: { routing: 1, pricing: 1 },
        id: expectedId,
        name: '',
        endpoint: 'https://new-provider.example/v1/',
        protocol: 'openai_chat',
        enabled: false,
        contexts: [],
        attachments: [],
        secret: 'synthetic-create-key',
      })
      return {
        data: { success: true, data: { version: { routing: 2, pricing: 1 } } },
      }
    }) as typeof api.request
    const container = await mount('channels')
    await until(() =>
      [...container.querySelectorAll('button')].some(
        (item) => item.textContent === 'Create Channel'
      )
    )
    await act(async () => button(container, 'Create Channel').click())
    const textarea = required(container.querySelector('textarea'))
    const draft = parseNativeDocument(textarea.value)
    assert.equal(draft.id, expectedId)
    assert.equal(Object.hasOwn(draft, 'channel_id_high_water'), false)
    draft.endpoint = 'https://new-provider.example/v1/'
    const textareaSetter = required(
      Object.getOwnPropertyDescriptor(
        dom.HTMLTextAreaElement.prototype,
        'value'
      )?.set
    )
    const secret = required(container.querySelector('input[type=password]'))
    const secretSetter = required(
      Object.getOwnPropertyDescriptor(dom.HTMLInputElement.prototype, 'value')
        ?.set
    )
    await act(async () => {
      textareaSetter.call(textarea, JSON.stringify(draft))
      textarea.dispatchEvent(new Event('input', { bubbles: true }))
      secretSetter.call(secret, 'synthetic-create-key')
      secret.dispatchEvent(new Event('input', { bubbles: true }))
    })
    await act(async () => button(container, 'Save').click())
    await until(() => writes === 1 && !container.querySelector('textarea'))
    assert.equal(writes, 1)
    await dispose?.()
    dispose = undefined
  }
})
