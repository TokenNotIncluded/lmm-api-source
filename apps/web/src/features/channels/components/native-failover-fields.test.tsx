/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { Window } from 'happy-dom'

import type { NativeConfiguration } from '../native-api'

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
  HTMLInputElement: dom.HTMLInputElement,
  requestAnimationFrame: dom.requestAnimationFrame.bind(dom),
  cancelAnimationFrame: dom.cancelAnimationFrame.bind(dom),
})) {
  Object.defineProperty(globalThis, name, { configurable: true, value })
}
Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
  configurable: true,
  writable: true,
  value: true,
})
const { act, useState } = await import('react')
const { createRoot } = await import('react-dom/client')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { NativeModelFields } = await import('./native-model-fields')
const { parseNativeDocument, stringifyNativeDocument } =
  await import('../native-api')
const i18n = createInstance()
await i18n.use(initReactI18next).init({
  lng: 'en',
  resources: { en: { translation: {} } },
  keySeparator: false,
  nsSeparator: false,
})
let dispose: (() => Promise<void>) | undefined
afterEach(async () => {
  await dispose?.()
  dispose = undefined
  document.body.replaceChildren()
})
after(() => dom.close())

function fixture() {
  return {
    expected_version: { routing: 7, pricing: 3 },
    route: {
      id: 1,
      model: 'public-model',
      group: 'default',
      enabled: false,
      speeds: ['standard'],
      targets: [],
      failover: {
        max_attempts: 1,
        retry_on: [] as string[],
        allow_replay_after_send: false,
      },
    },
    metadata: {
      display_name: 'Public model',
      description: 'Synthetic configuration',
      vendor_id: null,
    },
    prices: [
      {
        id: 1,
        model: 'public-model',
        group: 'default',
        speed: 'standard',
        rates: {
          input: 1_000_001,
          output: 2_000_000,
          cache_read: 0,
          cache_write: 0,
          request: 0,
        },
        group_multiplier_ppm: 1_000_000,
        speed_multiplier_ppm: 1_000_000,
      },
    ],
    contexts: [],
  }
}
async function mount(initial = fixture()) {
  const configuration: NativeConfiguration = {
    version: initial.expected_version,
    secret_storage_enabled: true,
    routing: {
      version: 7,
      upstreams: [],
      groups: [{ name: 'default', enabled: true }],
      routes: [],
      aliases: [],
    },
    prices: { version: 3, rules: initial.prices },
    bindings: {
      names: {},
      models: {},
      contexts: [],
      vendors: {},
      discoveries: {},
      price_drafts: {},
    },
  }
  const changes: (typeof initial)[] = []
  function Editor() {
    const [document, setDocument] = useState(
      stringifyNativeDocument(initial, 2)
    )
    return (
      <form>
        <NativeModelFields
          document={document}
          configuration={configuration}
          disabled={false}
          onChange={(next) => {
            changes.push(parseNativeDocument(next) as typeof initial)
            setDocument(next)
          }}
        />
      </form>
    )
  }
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  await act(async () =>
    root.render(
      <I18nextProvider i18n={i18n}>
        <Editor />
      </I18nextProvider>
    )
  )
  dispose = async () => {
    await act(async () => root.unmount())
  }
  const section = container.querySelector<HTMLElement>(
    'section[aria-label="Failover policy"]'
  )
  assert.ok(section)
  const attempts = section.querySelector<HTMLInputElement>('input[type=number]')
  assert.ok(attempts)
  return { container, section, attempts, changes }
}
async function input(element: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(
    HTMLInputElement.prototype,
    'value'
  )?.set
  assert.ok(setter)
  await act(async () => {
    setter.call(element, value)
    element.dispatchEvent(new Event('input', { bubbles: true }))
  })
}
function checkbox(section: HTMLElement, text: string) {
  const label = [...section.querySelectorAll('label')].find(
    (item) => item.textContent?.trim() === text
  )
  const control = label?.querySelector<HTMLElement>('[role=checkbox]')
  assert.ok(control, text)
  return control
}
async function toggle(section: HTMLElement, text: string) {
  const control = checkbox(section, text)
  const input = control
    .closest('label')
    ?.querySelector<HTMLInputElement>('input[type=checkbox]')
  assert.ok(input)
  await act(async () => input.click())
}

test('model failover edits preserve CAS, exact prices and safe defaults in the same document', async () => {
  const initial = fixture()
  const { section, attempts, changes } = await mount(initial)
  assert.equal(attempts.value, '1')
  assert.equal(
    checkbox(section, 'Allow replay after send').getAttribute('aria-checked'),
    'false'
  )
  assert.equal(changes.length, 0)
  assert.match(section.textContent ?? '', /Includes the first attempt/)
  assert.match(section.textContent ?? '', /duplicate work and charges/)
  assert.match(
    section.textContent ?? '',
    /current public API requests do not grant/
  )
  await input(attempts, '3')
  assert.match(
    section.querySelector('[role=status]')?.textContent ?? '',
    /Select at least one/
  )
  await toggle(section, 'Connection failure')
  await toggle(section, 'Rate limit (429)')
  assert.equal(section.querySelector('[role=status]') === null, true)
  const changed = changes.at(-1)
  assert.ok(changed)
  assert.deepEqual(changed.route.failover, {
    max_attempts: 3,
    retry_on: ['connect', 'rate_limited'],
    allow_replay_after_send: false,
  })
  assert.deepEqual(
    {
      ...changed,
      route: { ...changed.route, failover: initial.route.failover },
    },
    initial
  )
})

test('existing retry causes survive attempt edits and replay requires its own explicit action', async () => {
  const initial = fixture()
  initial.route.failover = {
    max_attempts: 4,
    retry_on: ['timeout', 'server_error'],
    allow_replay_after_send: false,
  }
  const { section, attempts, changes } = await mount(initial)
  assert.equal(
    checkbox(section, 'Timeout').getAttribute('aria-checked'),
    'true'
  )
  assert.equal(
    checkbox(section, 'Server error (5xx)').getAttribute('aria-checked'),
    'true'
  )
  assert.equal(section.querySelectorAll('[role=checkbox]').length, 5)
  await input(attempts, '1')
  assert.deepEqual(changes.at(-1)?.route.failover, {
    ...initial.route.failover,
    max_attempts: 1,
  })
  await toggle(section, 'Allow replay after send')
  assert.equal(changes.at(-1)?.route.failover.allow_replay_after_send, true)
  await toggle(section, 'Timeout')
  assert.deepEqual(changes.at(-1)?.route.failover.retry_on, ['server_error'])
  await toggle(section, 'Allow replay after send')
  assert.equal(changes.at(-1)?.route.failover.allow_replay_after_send, false)
})

test('attempt limits match Core and incomplete numeric edits keep the model form visible', async () => {
  const { container, section, attempts, changes } = await mount()
  assert.equal(attempts.min, '1')
  assert.equal(attempts.max, '8')
  assert.equal(attempts.step, '1')
  for (const value of ['', '0', '9', '1.5']) {
    await input(attempts, value)
    assert.equal(attempts.value, value)
    assert.equal(attempts.checkValidity(), false, value)
    assert.ok(container.querySelector('[data-slot=native-model-fields]'))
    assert.equal(changes.at(-1)?.route.failover.allow_replay_after_send, false)
  }
  await input(attempts, '8')
  assert.equal(attempts.checkValidity(), true)
  assert.equal(changes.at(-1)?.route.failover.max_attempts, 8)
  assert.equal(
    checkbox(section, 'Allow replay after send').getAttribute('aria-checked'),
    'false'
  )
})
