/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { Window } from 'happy-dom'

const dom = new Window({
  settings: { device: { prefersReducedMotion: 'reduce' } },
})
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
  HTMLTextAreaElement: dom.HTMLTextAreaElement,
  DocumentFragment: dom.DocumentFragment,
  ShadowRoot: dom.ShadowRoot,
  requestAnimationFrame: dom.requestAnimationFrame.bind(dom),
  cancelAnimationFrame: dom.cancelAnimationFrame.bind(dom),
  getComputedStyle: dom.getComputedStyle.bind(dom),
  scrollTo: dom.scrollTo.bind(dom),
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
const { readNativeRoutingScope } = await import('../../native-api')
const { NativeKeyManageDialog } = await import('./native-key-manage-dialog')

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
  useAuthStore.setState({ auth: previous })
  document.body.replaceChildren()
})
after(() => dom.close())

function required<T>(value: T | null | undefined): T {
  assert.ok(value)
  return value
}
function button(container: Element, text: string) {
  return required(
    [...container.querySelectorAll('button')].find(
      (element) => element.textContent === text
    )
  )
}
async function until(check: () => boolean) {
  for (let attempt = 0; attempt < 100; attempt++) {
    if (check()) return
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 10))
    })
  }
  assert.ok(check(), 'The key dialog did not reach the expected state')
}
async function changeInput(
  input: HTMLInputElement | HTMLTextAreaElement,
  value: string
) {
  const prototype =
    input.tagName === 'TEXTAREA'
      ? dom.HTMLTextAreaElement.prototype
      : dom.HTMLInputElement.prototype
  const setter = required(
    Object.getOwnPropertyDescriptor(prototype, 'value')?.set
  )
  await act(async () => {
    setter.call(input, value)
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
}
function keyResponses() {
  const writes: Array<Record<string, unknown>> = []
  let version = 1
  api.request = (async (options: {
    method: string
    url: string
    data?: Record<string, unknown>
  }) => {
    assert.equal(options.url, '/core/v1/admin/routing/channels/1/keys')
    if (options.method === 'GET') {
      return {
        data: {
          success: true,
          data: {
            version: { routing: version, pricing: 1 },
            active_reference: { id: 7, revision: 1 },
            polling: false,
            keys: [
              {
                reference: { id: 7, revision: 1 },
                label: 'Primary',
                created_at: '2026-11-16T00:00:00Z',
                enabled: true,
              },
            ],
            plaintext_available: false,
          },
        },
      }
    }
    assert.equal(options.method, 'POST')
    const data = required(options.data)
    writes.push(data)
    version++
    return {
      data: {
        success: true,
        data: {
          version: { routing: version, pricing: 1 },
          reference: { id: 7, revision: 1 },
          ...(data.operation === 'add_many' ? { imported_count: 2 } : {}),
        },
      },
    }
  }) as typeof api.request
  return writes
}
async function mount(bulkImportAllowed?: boolean) {
  useAuthStore.getState().auth.setBundle({
    access_token: 'synthetic-key-session-token',
    token_type: 'Bearer',
    access_expires_at: 2_000_000_000,
    user: { id: 501, role: 10, username: 'synthetic-admin' },
    session: {
      sid: 'synthetic-key-capabilities',
      current: true,
      login_method: 'password',
      ip: '127.0.0.1',
      user_agent: 'synthetic',
      created_at: 1,
      last_active_at: 1,
      expires_at: 2_000_000_000,
    },
  })
  const scope = required(readNativeRoutingScope())
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  dispose = async () => {
    await act(async () => root.unmount())
  }
  const render = async (allowed?: boolean) => {
    await act(async () => {
      root.render(
        <I18nextProvider i18n={i18n}>
          <NativeKeyManageDialog
            open
            onOpenChange={() => {}}
            scope={scope}
            channelId={1}
            channelName='Synthetic provider'
            bulkImportAllowed={allowed}
          />
        </I18nextProvider>
      )
    })
  }
  await render(bulkImportAllowed)
  await until(
    () =>
      !!document
        .querySelector('[role=dialog]')
        ?.textContent?.includes('Primary')
  )
  return {
    dialog: required(document.querySelector('[role=dialog]')),
    render,
  }
}

for (const capability of [undefined, false]) {
  test(`single-key addition remains available when bulk import is ${capability === undefined ? 'unadvertised' : 'disabled'}`, async () => {
    const writes = keyResponses()
    const { dialog } = await mount(capability)
    assert.equal(
      [...dialog.querySelectorAll('button')].some(
        (element) => element.textContent === 'Batch import'
      ),
      false
    )
    assert.equal(dialog.querySelector('textarea'), null)
    const form = required(dialog.querySelector('form'))
    await changeInput(required(form.querySelector('input')), 'Single')
    await changeInput(
      required(form.querySelector<HTMLInputElement>('input[type=password]')),
      'synthetic-single'
    )
    await act(async () => button(form, 'Add').click())
    await until(() => writes.length === 1 && !button(form, 'Add').disabled)
    assert.deepEqual(writes, [
      {
        expected_version: { routing: 1, pricing: 1 },
        operation: 'add',
        label: 'Single',
        secret: 'synthetic-single',
        activate: false,
      },
    ])
  })
}

test('advertised bulk import keeps one atomic request and its versioned receipt', async () => {
  const writes = keyResponses()
  const { dialog } = await mount(true)
  await act(async () => button(dialog, 'Batch import').click())
  const form = required(dialog.querySelector('form'))
  await changeInput(required(form.querySelector('input')), 'Pool')
  const secrets = required(form.querySelector('textarea'))
  await changeInput(secrets, 'synthetic-first\nsynthetic-second')
  await act(async () => {
    button(form, 'Import keys').click()
    button(form, 'Import keys').click()
  })
  await until(
    () => writes.length === 1 && !button(form, 'Import keys').disabled
  )
  assert.deepEqual(writes, [
    {
      expected_version: { routing: 1, pricing: 1 },
      operation: 'add_many',
      keys: [
        { label: 'Pool 1', secret: 'synthetic-first' },
        { label: 'Pool 2', secret: 'synthetic-second' },
      ],
      polling: false,
    },
  ])
  assert.match(dialog.textContent ?? '', /Imported: 2/)
  assert.equal(secrets.value, '')
})

test('revoking bulk capability discards the draft without turning it into a single-key write', async () => {
  const writes = keyResponses()
  const { dialog, render } = await mount(true)
  await act(async () => button(dialog, 'Batch import').click())
  const form = required(dialog.querySelector('form'))
  await changeInput(required(form.querySelector('input')), 'Pool')
  await changeInput(
    required(form.querySelector('textarea')),
    'synthetic-unsent-first\nsynthetic-unsent-second'
  )
  await render(false)
  assert.equal(dialog.querySelector('textarea'), null)
  assert.equal(
    required(form.querySelector<HTMLInputElement>('input[type=password]'))
      .value,
    ''
  )
  await act(async () => {
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
  })
  assert.equal(writes.length, 0)
  await render(true)
  await act(async () => button(dialog, 'Batch import').click())
  assert.equal(required(form.querySelector('textarea')).value, '')
  assert.equal(writes.length, 0)
})
