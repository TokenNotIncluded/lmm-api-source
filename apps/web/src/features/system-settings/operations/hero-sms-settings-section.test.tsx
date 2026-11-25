/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { AxiosError } from 'axios'
import { Window } from 'happy-dom'

const dom = new Window({
  url: 'https://console.example.test/system-settings/operations/hero-sms',
})
Reflect.deleteProperty(dom.Element.prototype, 'animate')
for (const name of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLInputElement',
  'HTMLButtonElement',
  'Node',
  'Element',
  'Event',
  'MouseEvent',
  'MutationObserver',
  'ResizeObserver',
  'DocumentFragment',
  'ShadowRoot',
  'getComputedStyle',
  'matchMedia',
  'requestAnimationFrame',
  'cancelAnimationFrame',
] as const) {
  Object.defineProperty(globalThis, name, {
    configurable: true,
    value: dom[name],
  })
}
Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
  configurable: true,
  value: true,
})

const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider, notifyManager } =
  await import('@tanstack/react-query')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { SettingsPageProvider } =
  await import('../components/settings-page-context')
const { HeroSmsSettingsSection } = await import('./hero-sms-settings-section')
notifyManager.setScheduler(queueMicrotask)
const i18n = createInstance()
await i18n.use(initReactI18next).init({ lng: 'en', resources: {} })
const priorAdapter = api.defaults.adapter
let dispose: (() => Promise<void>) | undefined

afterEach(async () => {
  await dispose?.()
  dispose = undefined
  api.defaults.adapter = priorAdapter
  document.body.replaceChildren()
})
after(() => dom.close())

const settings = {
  revision: '9007199254740993',
  enabled: true,
  email_enabled: true,
  sms_enabled: false,
  api_key_configured: true,
  credential_managed_by_file: true,
  pending_work: false,
  purchases_available: false,
  currency: 'USD',
  currency_code: 840,
  price_multiplier: '10',
}

async function mount() {
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: 1, retryDelay: 0 },
    },
  })
  const container = document.createElement('div')
  const actions = document.createElement('div')
  document.body.append(container, actions)
  const root = createRoot(container)
  await act(async () =>
    root.render(
      <QueryClientProvider client={client}>
        <I18nextProvider i18n={i18n}>
          <SettingsPageProvider actionsContainer={actions}>
            <HeroSmsSettingsSection />
          </SettingsPageProvider>
        </I18nextProvider>
      </QueryClientProvider>
    )
  )
  dispose = async () => {
    await act(async () => root.unmount())
    client.clear()
  }
  await until(
    () =>
      container.textContent?.includes(
        'HeroSMS API key is managed on the host'
      ) === true
  )
  assert.ok(container.textContent?.includes('Purchases unavailable'))
  return { container, actions }
}

async function until(check: () => boolean) {
  for (let attempt = 0; attempt < 50 && !check(); attempt++) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 10))
    })
  }
  assert.ok(check(), 'HeroSMS settings did not reach the expected state')
}

function button(text: string) {
  const result = Array.from(
    document.querySelectorAll<HTMLButtonElement>('button')
  ).find((element) => element.textContent?.trim() === text)
  assert.ok(result, `Missing button: ${text}`)
  return result
}

async function toggle(label: string) {
  const fieldLabel = Array.from(document.querySelectorAll('label')).find(
    (element) => element.textContent === label
  )
  assert.ok(fieldLabel)
  const control = document.getElementById(fieldLabel.htmlFor)
  assert.ok(control)
  await act(async () => control.click())
}

test('host-managed credentials remain testable while stale activation settings require an explicit reload', async () => {
  const writes: Record<string, unknown>[] = []
  const tests: unknown[] = []
  let reads = 0
  api.defaults.adapter = async (config) => {
    if (config.method === 'get') reads++
    if (config.method === 'put') {
      writes.push(JSON.parse(config.data))
      if (writes.length === 1) {
        throw new AxiosError(
          'stale revision',
          'ERR_BAD_REQUEST',
          config,
          undefined,
          {
            config,
            headers: {},
            status: 409,
            statusText: 'Conflict',
            data: { success: false, code: 'STATE_CONFLICT' },
          }
        )
      }
    }
    if (config.method === 'post') tests.push(JSON.parse(config.data))
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: {
        success: true,
        data:
          config.method === 'post'
            ? { ok: true }
            : {
                ...settings,
                revision: reads === 1 ? settings.revision : '9007199254740995',
              },
      },
    }
  }
  const { container } = await mount()
  assert.equal(container.querySelector('input[type="password"]'), null)
  assert.equal(document.body.textContent?.includes('Clear saved key'), false)
  assert.equal(
    document.body.textContent?.includes('Replacement API key'),
    false
  )
  await act(async () => button('Test connection').click())
  await until(
    () =>
      container.textContent?.includes(
        'The server can reach HeroSMS with the configured host credential.'
      ) === true
  )
  assert.deepEqual(tests, [{}])

  await toggle('Enable phone-number activations')
  await act(async () => button('Save Changes').click())
  await until(
    () =>
      container.textContent?.includes(
        'HeroSMS settings changed. Reload the latest settings before saving again.'
      ) === true
  )
  assert.equal(writes.length, 1)
  assert.equal(writes[0].expected_revision, settings.revision)
  assert.equal(writes[0].sms_enabled, true)
  assert.equal('api_key' in writes[0], false)
  assert.equal(
    reads,
    1,
    'Conflict must not silently load a revision and retry the write'
  )

  await act(async () => button('Reload').click())
  await until(
    () =>
      reads === 2 &&
      !container.textContent?.includes(
        'HeroSMS settings changed. Reload the latest settings before saving again.'
      )
  )
  await toggle('Enable email activations')
  await act(async () => button('Save Changes').click())
  await until(() => writes.length === 2 && button('Save Changes').disabled)
  assert.equal(writes[1].expected_revision, '9007199254740995')
  assert.equal(writes[1].email_enabled, false)
})

test('an unconfigured host credential cannot be tested from the browser', async () => {
  api.defaults.adapter = async (config) => ({
    config,
    headers: {},
    status: 200,
    statusText: 'OK',
    data: {
      success: true,
      data: { ...settings, api_key_configured: false, enabled: false },
    },
  })
  await mount()
  assert.equal(button('Test connection').disabled, true)
})

test('Core file policy shows the returned settings and tests only the host credential', async () => {
  const calls: [string | undefined, string | undefined][] = []
  api.defaults.adapter = async (config) => {
    calls.push([config.method, config.url])
    if (config.method === 'post') assert.deepEqual(JSON.parse(config.data), {})
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: {
        success: true,
        data:
          config.method === 'post'
            ? { ok: true }
            : {
                ...settings,
                policy_managed_by_file: true,
                price_multiplier: '12.500000',
              },
      },
    }
  }
  const { container } = await mount()
  assert.ok(container.textContent?.includes('hero_sms.policy_file'))
  assert.ok(container.textContent?.includes('12.500000'))
  assert.equal(document.querySelector('form, [role="switch"], input'), null)
  assert.equal(document.body.textContent?.includes('Save Changes'), false)
  await act(async () => button('Test connection').click())
  await until(
    () => container.textContent?.includes('Connection test succeeded') === true
  )
  assert.deepEqual(calls, [
    ['get', '/api/option/hero-sms'],
    ['post', '/api/option/hero-sms/test'],
  ])
})
