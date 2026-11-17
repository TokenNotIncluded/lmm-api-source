/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { Window } from 'happy-dom'

const dom = new Window({
  url: 'https://console.example.test/system-settings/auth/custom-oauth',
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
const { CustomOAuthSection } = await import('../custom-oauth-section')
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

const provider = {
  id: '9007199254740993',
  revision: '9007199254740995',
  kind: 'custom_oauth',
  name: 'Company login',
  slug: 'company',
  icon: '',
  enabled: true,
  client_id: 'synthetic-client',
  client_secret: '',
  secret_configured: true,
  authorization_endpoint: 'https://identity.example.test/authorize',
  token_endpoint: 'https://identity.example.test/token',
  user_info_endpoint: 'https://identity.example.test/userinfo',
  scopes: 'profile',
  user_id_field: 'sub',
  username_field: 'name',
  display_name_field: 'name',
  email_field: 'email',
  well_known: '',
  auth_style: 2,
  pkce_s256: true,
  access_policy: '',
  access_denied_message: '',
}

async function mount() {
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: 1, retryDelay: 0 },
    },
  })
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  await act(async () =>
    root.render(
      <QueryClientProvider client={client}>
        <I18nextProvider i18n={i18n}>
          <CustomOAuthSection serverAddress='https://console.example.test' />
        </I18nextProvider>
      </QueryClientProvider>
    )
  )
  dispose = async () => {
    await act(async () => root.unmount())
    client.clear()
  }
  await until(() => container.textContent?.includes(provider.name) === true)
  return container
}

async function until(check: () => boolean) {
  for (let attempt = 0; attempt < 50 && !check(); attempt++) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 10))
    })
  }
  assert.ok(check(), 'Provider settings did not reach the expected state')
}

test('a lost save response locks the draft and explicitly retries the same operation', async () => {
  const writes: Record<string, unknown>[] = []
  api.defaults.adapter = async (config) => {
    if (config.method === 'put') {
      writes.push(JSON.parse(config.data))
      if (writes.length === 1) throw new Error('Connection interrupted')
    }
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: {
        success: true,
        data: config.method === 'put' ? provider : [provider],
      },
    }
  }
  const container = await mount()
  const edit = container.querySelector<HTMLButtonElement>(
    'button[aria-label="Edit"]'
  )
  assert.ok(edit)
  await act(async () => edit.click())
  await until(
    () => document.querySelector('#custom-oauth-provider-form') !== null
  )
  assert.ok(
    document.body.textContent?.includes('Leave blank to keep the saved secret')
  )
  const form = document.querySelector<HTMLFormElement>(
    '#custom-oauth-provider-form'
  )
  assert.ok(form)
  await act(async () => {
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
  })
  const retryButton = () =>
    Array.from(document.querySelectorAll<HTMLButtonElement>('button')).find(
      (button) => button.textContent?.trim() === 'Retry saved request'
    )
  await until(() => !!retryButton())
  assert.equal(
    writes.length,
    1,
    'Mutation defaults must not replay a provider write'
  )
  assert.equal(form.querySelector('fieldset')?.disabled, true)
  assert.equal(writes[0].pkce_s256, true)
  assert.equal(writes[0].expected_revision, provider.revision)
  assert.equal(writes[0].client_secret, '')
  assert.equal('secret_configured' in writes[0], false)
  const retry = retryButton()
  assert.ok(retry)
  await act(async () => retry.click())
  await until(
    () =>
      writes.length === 2 &&
      document.querySelector('#custom-oauth-provider-form') === null
  )
  assert.deepEqual(writes[1], writes[0])
})

test('a rejected deletion stays open and does not report an unknown result', async () => {
  let deletes = 0
  api.defaults.adapter = async (config) => {
    if (config.method === 'delete') deletes++
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data:
        config.method === 'delete'
          ? { success: false, message: 'Provider has linked accounts' }
          : { success: true, data: [provider] },
    }
  }
  const container = await mount()
  const menu = container.querySelector<HTMLButtonElement>(
    'button[aria-label="Open menu"]'
  )
  assert.ok(menu)
  await act(async () => menu.click())
  await until(() => document.querySelector('[role="menuitem"]') !== null)
  const deleteAction = Array.from(
    document.querySelectorAll<HTMLElement>('[role="menuitem"]')
  ).find((item) => item.textContent?.trim() === 'Delete')
  assert.ok(deleteAction)
  await act(async () => deleteAction.click())
  await until(() => document.querySelector('[role="alertdialog"]') !== null)
  const dialog = document.querySelector('[role="alertdialog"]')
  assert.ok(dialog)
  assert.ok(
    dialog.textContent?.includes(
      'Providers with linked accounts cannot be deleted.'
    )
  )
  const confirm = Array.from(
    dialog.querySelectorAll<HTMLButtonElement>('button')
  ).find((button) => button.textContent?.trim() === 'Delete')
  assert.ok(confirm)
  await act(async () => confirm.click())
  await until(() => deletes === 1 && !confirm.disabled)
  assert.ok(document.querySelector('[role="alertdialog"]'))
  assert.equal(dialog.textContent?.includes('Retry saved request'), false)
})
