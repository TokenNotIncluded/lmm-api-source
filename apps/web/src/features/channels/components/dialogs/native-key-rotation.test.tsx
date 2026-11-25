/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { Window } from 'happy-dom'
import type { ComponentProps } from 'react'

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
  HTMLButtonElement: dom.HTMLButtonElement,
  IS_REACT_ACT_ENVIRONMENT: true,
})) {
  Object.defineProperty(globalThis, name, {
    configurable: true,
    writable: true,
    value,
  })
}
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { NativeKeyRotation } = await import('./native-key-rotation')
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
type Props = ComponentProps<typeof NativeKeyRotation>
let root: ReturnType<typeof createRoot>
afterEach(async () => {
  await act(async () => root.unmount())
  document.body.replaceChildren()
})
after(() => dom.close())

async function show(props: Partial<Props> = {}) {
  const container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
  const base: Props = {
    channelId: 1,
    endpoint: 'https://upstream.example.test',
    protocol: 'openai_chat',
    activeReference: { id: 9, revision: 2 },
    disabled: false,
    async onVerify() {},
    async onSwitch() {},
    ...props,
  }
  async function render(update: Partial<Props> = {}) {
    await act(async () =>
      root.render(
        <I18nextProvider i18n={i18n}>
          <NativeKeyRotation {...base} {...update} />
        </I18nextProvider>
      )
    )
  }
  await render()
  const buttons = () => container.querySelectorAll('button')
  const inputs = () => container.querySelectorAll('input')
  async function write(index: number, value: string) {
    const setter = Object.getOwnPropertyDescriptor(
      dom.HTMLInputElement.prototype,
      'value'
    )?.set
    assert.ok(setter)
    await act(async () => {
      const input = inputs()[index]
      setter.call(input, value)
      input.dispatchEvent(new Event('input', { bubbles: true }))
    })
  }
  async function fill() {
    await write(0, 'New key')
    await write(1, 'synthetic-replacement')
  }
  return { container, render, buttons, inputs, write, fill }
}

test('verifying has no write effect and a separate double confirmation dispatches only once', async () => {
  const checks: unknown[] = []
  const writes: unknown[] = []
  const { buttons, inputs, fill } = await show({
    async onVerify(candidate) {
      checks.push(candidate)
    },
    async onSwitch(candidate) {
      writes.push(candidate)
    },
  })
  await fill()
  assert.equal(buttons()[1].disabled, true)
  await act(async () => buttons()[0].click())
  assert.deepEqual(checks, [
    {
      endpoint: 'https://upstream.example.test',
      protocol: 'openai_chat',
      secret: 'synthetic-replacement',
    },
  ])
  assert.equal(inputs()[1].value, 'synthetic-replacement')
  assert.deepEqual(writes, [])
  await act(async () => {
    buttons()[1].click()
    buttons()[1].click()
  })
  assert.deepEqual(writes, [
    { label: 'New key', secret: 'synthetic-replacement' },
  ])
  assert.equal(inputs()[1].value, '')
  assert.equal(buttons()[1].disabled, true)
})

test('a late verification cannot approve an edited key or a different channel configuration', async () => {
  let finish!: () => void
  let signal!: AbortSignal
  const { buttons, inputs, write, fill, render } = await show({
    async onVerify(_candidate, currentSignal) {
      signal = currentSignal
      await new Promise<void>((resolve) => {
        finish = resolve
      })
    },
    async onSwitch() {
      assert.fail('unverified key was switched')
    },
  })
  await fill()
  await act(async () => buttons()[0].click())
  await write(1, 'synthetic-edited')
  assert.equal(signal.aborted, true)
  await act(async () => finish())
  assert.equal(buttons()[1].disabled, true)
  for (const change of [
    { endpoint: 'https://another.example.test' },
    { protocol: 'anthropic_messages' },
    { activeReference: { id: 9, revision: 3 } },
    { channelId: 8 },
    { disabled: true },
  ]) {
    await render()
    await fill()
    await act(async () => buttons()[0].click())
    await render(change)
    assert.equal(signal.aborted, true)
    await act(async () => finish())
    assert.equal(inputs()[1].value, '')
    assert.equal(buttons()[1].disabled, true)
  }
  await render()
  await fill()
  await act(async () => buttons()[0].click())
  await act(async () => root.render(null))
  assert.equal(signal.aborted, true)
  await act(async () => finish())
  await render()
  assert.equal(inputs()[1].value, '')
  assert.equal(buttons()[1].disabled, true)
})

test('secret-bearing errors stay out of the page and an uncertain switch cannot be replayed', async () => {
  let rejectProbe = true
  let count = 0
  const { buttons, inputs, container, fill } = await show({
    async onVerify({ secret }) {
      if (rejectProbe) throw new Error(secret)
    },
    async onSwitch({ secret }) {
      count++
      throw new Error(secret)
    },
  })
  await fill()
  await act(async () => buttons()[0].click())
  assert.match(
    container.querySelector('[role=alert]')?.textContent ?? '',
    /could not be verified/
  )
  assert.equal(container.textContent?.includes('synthetic-replacement'), false)
  assert.equal(inputs()[1].value, 'synthetic-replacement')
  rejectProbe = false
  await act(async () => buttons()[0].click())
  await act(async () => buttons()[1].click())
  assert.equal(count, 1)
  assert.match(
    container.querySelector('[role=alert]')?.textContent ?? '',
    /Check the active key/
  )
  assert.equal(container.textContent?.includes('synthetic-replacement'), false)
  assert.equal(inputs()[1].value, '')
  assert.equal(buttons()[0].disabled, true)
  assert.equal(buttons()[1].disabled, true)
})

test('file-managed verification clears the draft secret and never offers activation', async () => {
  const probes: string[] = []
  const { buttons, inputs, write, container } = await show({
    onSwitch: undefined,
    async onVerify({ secret }) {
      probes.push(secret)
    },
  })
  assert.equal(inputs().length, 1)
  await write(0, 'synthetic-vault-draft')
  await act(async () => buttons()[0].click())
  assert.deepEqual(probes, ['synthetic-vault-draft'])
  assert.equal(inputs()[0].value, '')
  assert.equal(buttons().length, 1)
  assert.equal(container.textContent?.includes('Confirm key switch'), false)
  assert.match(container.textContent ?? '', /Connection successful/)
})
