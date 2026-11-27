/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, beforeEach, test } from 'node:test'

import { AxiosError } from 'axios'
import { Window } from 'happy-dom'

const dom = new Window({
  url: 'https://console.example.test/extensions/plugins?configure=assistant',
})
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLInputElement',
  'HTMLSelectElement',
  'Node',
  'Event',
  'MouseEvent',
  'Element',
  'MutationObserver',
  'localStorage',
  'customElements',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: key === 'window' ? dom : dom[key],
  })
}
Object.assign(globalThis, {
  IS_REACT_ACT_ENVIRONMENT: true,
  scrollTo: dom.scrollTo.bind(dom),
})
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const { createRootRoute, createRouter, createMemoryHistory, RouterProvider } =
  await import('@tanstack/react-router')
const { createInstance } = await import('i18next')
const { I18nextProvider } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { AssistantPluginSettings } = await import('./assistant-plugin-settings')
const i18n = createInstance()
await i18n.init({ lng: 'en', resources: { en: { translation: {} } } })
const originalAdapter = api.defaults.adapter
const previousAuth = useAuthStore.getState().auth
const requests: Array<{ url: string; method: string; data: unknown }> = []
const hostModelPath = '/api/option/assistant-model'
const pluginPath = '/api/assistant/settings/model-route'
let pref = { group: '', model: '', revision: 0 }
function modelSnapshot() {
  return {
    model: 'first',
    default_model: 'first',
    group: 'support',
    models: ['first', 'second'],
    routes: { support: ['first', 'second'], private: ['team-model'] } as Record<
      string,
      string[]
    >,
    version: { routing: 3, pricing: 7 },
  }
}
let hostModel = modelSnapshot()
let hostReadFails = false
let hostWriteFails = false
let hostSaveForbidden = false

beforeEach(() => {
  pref = { group: '', model: '', revision: 0 }
  hostModel = modelSnapshot()
  hostReadFails = false
  hostWriteFails = false
  hostSaveForbidden = false
  requests.length = 0
  useAuthStore.getState().auth.setBundle({
    access_token: 'synthetic-admin',
    token_type: 'Bearer',
    access_expires_at: 2000000000,
    user: { id: 101, username: 'admin', role: 100 },
    session: {
      sid: 'synthetic-admin-session',
      current: true,
      login_method: 'password',
      ip: '',
      user_agent: '',
      created_at: 1,
      last_active_at: 1,
      expires_at: 2000000000,
    },
  })
  api.defaults.adapter = async (config) => {
    const data = config.data ? JSON.parse(String(config.data)) : undefined
    requests.push({ url: config.url ?? '', method: config.method ?? '', data })
    if (config.url === pluginPath) {
      if (config.method === 'put') {
        assert.deepEqual(Object.keys(data).sort(), [
          'group',
          'model',
          'revision',
        ])
        assert.equal(data.revision, pref.revision)
        pref = {
          group: data.group,
          model: data.model,
          revision: pref.revision + 1,
        }
      }
      return {
        config,
        headers: {},
        status: 200,
        statusText: 'OK',
        data: { success: true, data: pref },
      }
    }
    if (config.url === hostModelPath) {
      assert.equal(config.skipAuthRefresh, true)
      assert.equal(
        config.authScope?.userId,
        useAuthStore.getState().auth.user?.id
      )
      let status = 200
      if (config.method === 'get') {
        if (hostReadFails) status = 503
      } else {
        assert.equal(config.method, 'put')
        assert.deepEqual(Object.keys(data).sort(), [
          'expected_version',
          'group',
          'model',
        ])
        if (hostSaveForbidden) status = 403
        else if (hostWriteFails) status = 503
        else if (
          data.expected_version.routing !== hostModel.version.routing ||
          data.expected_version.pricing !== hostModel.version.pricing
        ) {
          status = 409
        } else {
          assert.ok(hostModel.routes[data.group]?.includes(data.model))
          hostModel = {
            ...hostModel,
            group: data.group,
            model: data.model,
            version: {
              ...hostModel.version,
              routing: hostModel.version.routing + 1,
            },
          }
        }
      }
      const response = {
        config,
        status,
        statusText: String(status),
        headers: {},
        data:
          status === 200
            ? { success: true, data: hostModel }
            : { success: false },
      }
      if (status !== 200) {
        throw new AxiosError(
          'Model settings request failed',
          undefined,
          config,
          undefined,
          response
        )
      }
      return response
    }

    let value: unknown
    if (config.url === '/api/assistant/status') {
      value = { enabled: true, route_available: true }
    } else if (
      config.url === '/api/assistant/settings/pre-conversation-presets'
    ) {
      value = {
        configuration:
          '[{"id":"custom","label":"Published label","prompt":"Published prompt"}]',
        revision: 1,
        updated_at: 10,
      }
    } else throw new Error(`Unexpected request to ${config.url}`)
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: { success: true, data: value },
    }
  }
})
afterEach(() => {
  api.defaults.adapter = originalAdapter
  useAuthStore.setState({ auth: previousAuth })
})
after(() => dom.close())
async function renderPlugin() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  const router = createRouter({
    routeTree: createRootRoute({ component: AssistantPluginSettings }),
    history: createMemoryHistory({ initialEntries: ['/'] }),
  })
  await router.load()
  await act(async () =>
    root.render(
      <QueryClientProvider client={client}>
        <I18nextProvider i18n={i18n}>
          <RouterProvider router={router} />
        </I18nextProvider>
      </QueryClientProvider>
    )
  )
  return {
    container,
    cleanup: async () => {
      await act(async () => root.unmount())
      client.clear()
      container.remove()
    },
  }
}
async function until(condition: () => boolean, label: string) {
  const deadline = Date.now() + 2000
  while (!condition() && Date.now() < deadline) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 10))
    })
  }
  assert.ok(condition(), label)
}
function button(container: HTMLElement, label: string) {
  const hit = [...container.querySelectorAll<HTMLButtonElement>('button')].find(
    (node) => node.textContent?.trim() === label
  )
  assert.ok(hit, label)
  return hit
}
async function click(container: HTMLElement, label: string) {
  await act(async () => button(container, label).click())
}
function field(container: HTMLElement) {
  const result = container.querySelector<HTMLInputElement>(
    'input[aria-label="Button label"]'
  )
  assert.ok(result)
  return result
}
test('the assistant plugin switches the host-group model only after Save, using the displayed version', async () => {
  const page = await renderPlugin()
  try {
    await until(
      () => Boolean(page.container.querySelector('#assistant-host-model')),
      'host model selector'
    )
    const select = page.container.querySelector<HTMLSelectElement>(
      '#assistant-host-model'
    )
    assert.ok(select)
    const section = select.closest('section')
    assert.ok(section)
    assert.ok(section.textContent?.includes('support'))
    assert.deepEqual(
      [...select.options].map((o) => o.value),
      ['first', 'second']
    )
    assert.equal(select.value, 'first')
    assert.equal(button(section, 'Save').disabled, true)
    await act(async () => {
      select.value = 'second'
      select.dispatchEvent(new Event('change', { bubbles: true }))
    })
    assert.equal(hostModel.model, 'first')
    await click(section, 'Save')
    await until(
      () => section.textContent?.includes('Saved') === true,
      'saved model'
    )
    assert.deepEqual(
      requests
        .filter((r) => r.url === hostModelPath && r.method === 'put')
        .map((r) => r.data),
      [
        {
          group: 'support',
          model: 'second',
          expected_version: { routing: 3, pricing: 7 },
        },
      ]
    )
    assert.deepEqual(pref, { group: 'support', model: 'second', revision: 1 })
    assert.equal(select.value, 'second')
    assert.equal(button(section, 'Save').disabled, true)
    assert.equal(field(page.container).value, 'Published label')
  } finally {
    await page.cleanup()
  }
})
test('host model read and save failures preserve conversation starters and a conflict requires an explicit reload', async () => {
  hostReadFails = true
  const page = await renderPlugin()
  try {
    await until(
      () =>
        page.container.textContent?.includes('Unable to load settings') ===
        true,
      'model read error'
    )
    assert.equal(field(page.container).value, 'Published label')
    const section = page.container
      .querySelector('label[for="assistant-host-model"]')
      ?.closest('section')
    assert.ok(section)
    hostReadFails = false
    await click(section, 'Reload configuration')
    await until(
      () => Boolean(section.querySelector('#assistant-host-model')),
      'model loaded'
    )
    const select = section.querySelector<HTMLSelectElement>(
      '#assistant-host-model'
    )
    assert.ok(select)
    await act(async () => {
      select.value = 'second'
      select.dispatchEvent(new Event('change', { bubbles: true }))
    })
    hostWriteFails = true
    await click(section, 'Save')
    await until(
      () => section.textContent?.includes('Save failed') === true,
      'model save error'
    )
    assert.equal(select.value, 'second')
    assert.deepEqual(pref, { group: 'support', model: 'second', revision: 1 })
    assert.equal(hostModel.model, 'first')
    await click(section, 'Reload configuration')
    await until(
      () => section.textContent?.includes('Pending') === true,
      'plugin preference pending publication'
    )
    hostWriteFails = false
    hostModel = { ...hostModel, version: { routing: 4, pricing: 9 } }
    await click(section, 'Save')
    await until(
      () =>
        section.textContent?.includes('Reload the current configuration') ===
        true,
      'model conflict'
    )
    assert.equal(button(section, 'Save').disabled, true)
    assert.equal(hostModel.model, 'first')
    await click(section, 'Reload configuration')
    await until(
      () => !button(section, 'Save').disabled,
      'pending preference reloaded'
    )
    await click(section, 'Save')
    await until(
      () => section.textContent?.includes('Saved') === true,
      'pending preference published'
    )
    assert.deepEqual(pref, { group: 'support', model: 'second', revision: 1 })
    assert.equal(hostModel.model, 'second')
    assert.equal(field(page.container).value, 'Published label')
  } finally {
    await page.cleanup()
  }
})

test('the plugin keeps a retired current model visible and withdraws the choices when access is denied', async () => {
  hostModel = {
    ...hostModel,
    model: 'retired',
    models: ['second'],
    routes: { support: ['second'] },
  }
  const page = await renderPlugin()
  try {
    await until(
      () => Boolean(page.container.querySelector('#assistant-host-model')),
      'retired model loaded'
    )
    const select = page.container.querySelector<HTMLSelectElement>(
      '#assistant-host-model'
    )
    assert.ok(select)
    const section = select.closest('section')
    assert.ok(section)
    assert.equal(select.value, 'retired')
    assert.equal(select.options[0].disabled, true)
    assert.deepEqual(
      [...select.options]
        .filter((option) => !option.disabled)
        .map((option) => option.value),
      ['second']
    )
    await act(async () => {
      select.value = 'second'
      select.dispatchEvent(new Event('change', { bubbles: true }))
    })
    hostSaveForbidden = true
    await click(section, 'Save')
    await until(
      () => section.textContent?.includes('Access Forbidden') === true,
      'model authorization rejected'
    )
    assert.equal(section.querySelector('#assistant-host-model'), null)
    assert.ok(!section.textContent?.includes('retired'))
    assert.equal(hostModel.model, 'retired')
    assert.equal(field(page.container).value, 'Published label')
  } finally {
    await page.cleanup()
  }
})

test('group selector publishes only models available in the saved group', async () => {
  const page = await renderPlugin()
  try {
    await until(
      () => Boolean(page.container.querySelector('#assistant-plugin-group')),
      'group selector'
    )
    const group = page.container.querySelector<HTMLSelectElement>(
      '#assistant-plugin-group'
    )!
    const model = page.container.querySelector<HTMLSelectElement>(
      '#assistant-host-model'
    )!
    const section = group.closest('section')!
    await act(async () => {
      group.value = 'private'
      group.dispatchEvent(new Event('change', { bubbles: true }))
    })
    assert.deepEqual(
      [...model.options].map((v) => v.value),
      ['team-model']
    )
    assert.equal(hostModel.group, 'support')
    await click(section, 'Save')
    await until(
      () => section.textContent?.includes('Saved') === true,
      'group and model published'
    )
    assert.deepEqual(pref, {
      group: 'private',
      model: 'team-model',
      revision: 1,
    })
    assert.equal(hostModel.group, pref.group)
    assert.equal(hostModel.model, pref.model)
    assert.deepEqual(
      requests.filter((r) => r.method === 'put').map((r) => r.url),
      [pluginPath, hostModelPath]
    )
  } finally {
    await page.cleanup()
  }
})
