/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { AxiosError } from 'axios'
import { Window } from 'happy-dom'

const dom = new Window({
  settings: { device: { prefersReducedMotion: 'reduce' } },
})
for (const name of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'Element',
  'Node',
  'Event',
  'MouseEvent',
  'Document',
  'DocumentFragment',
  'CustomEvent',
  'KeyboardEvent',
  'FocusEvent',
  'HTMLButtonElement',
  'HTMLInputElement',
  'MutationObserver',
  'ResizeObserver',
  'NodeFilter',
] as const) {
  Object.defineProperty(globalThis, name, {
    configurable: true,
    value: name === 'window' ? dom : dom[name],
  })
}
Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
  configurable: true,
  value: true,
})
for (const [name, value] of Object.entries({
  getComputedStyle: dom.getComputedStyle.bind(dom),
  requestAnimationFrame: dom.requestAnimationFrame.bind(dom),
  cancelAnimationFrame: dom.cancelAnimationFrame.bind(dom),
})) {
  Object.defineProperty(globalThis, name, { configurable: true, value })
}
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { PluginsPage } = await import('./index')
const { changePlugin, reloadPlugins } = await import('./api')
const { readNativeUsersScope } = await import('@/features/native-users/access')
const previousAuth = useAuthStore.getState().auth
const originalAdapter = api.defaults.adapter
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
let dispose: (() => Promise<void>) | undefined

function login(role: number, sessionId = 'synthetic-session') {
  useAuthStore.getState().auth.setBundle({
    access_token: 'synthetic-credential',
    token_type: 'Bearer',
    access_expires_at: 2000000000,
    user: { id: 901, role, username: 'synthetic-user' },
    session: {
      sid: sessionId,
      current: true,
      login_method: 'password',
      ip: '127.0.0.1',
      user_agent: 'synthetic',
      created_at: 1,
      last_active_at: 1,
      expires_at: 2000000000,
    },
  })
}

async function mount(configure?: string, section?: string) {
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  await act(async () =>
    root.render(
      <QueryClientProvider client={client}>
        <I18nextProvider i18n={i18n}>
          <PluginsPage configure={configure} section={section} />
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
  assert.ok(check(), 'Plugin page did not reach the expected state')
}

afterEach(async () => {
  await dispose?.()
  dispose = undefined
  api.defaults.adapter = originalAdapter
  useAuthStore.setState({ auth: previousAuth })
  document.body.replaceChildren()
})
after(() => dom.close())

test('administrator sees plugin state and refreshes with a scoped read only request', async () => {
  login(100)
  let calls = 0
  api.defaults.adapter = async (config) => {
    assert.equal(config.url, '/api/extensions/plugins')
    assert.equal(config.method, 'get')
    assert.equal(
      config.headers.get('Authorization'),
      'Bearer synthetic-credential'
    )
    assert.deepEqual(config.authScope, {
      userId: 901,
      sessionId: 'synthetic-session',
    })
    calls++
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: {
        success: true,
        data: [
          {
            name: 'store',
            version: '1.0.0',
            api_version: 1,
            registration_sha256: 'a'.repeat(64),
            available: true,
            installed: true,
            enabled: calls === 1,
          },
        ],
      },
    }
  }
  const container = await mount()
  await until(() => container.textContent?.includes('Enabled modules') === true)
  assert.match(container.textContent ?? '', /store/)
  assert.match(container.textContent ?? '', /Installed/)
  assert.match(
    container.textContent ?? '',
    /Registry entries are declarations, not service health checks/
  )
  assert.deepEqual(
    [...container.querySelectorAll('button')].map(
      (button) => button.textContent
    ),
    ['Refresh', 'Reload routes', 'Disable']
  )
  await act(async () => container.querySelector('button')?.click())
  await until(
    () => calls === 2 && container.querySelector('dd')?.textContent === '0'
  )
  assert.equal(container.querySelector('dd')?.textContent, '0')
})

test('non-admin cannot query, and invalid API responses render an error', async () => {
  let calls = 0
  api.defaults.adapter = async (config) => {
    calls++
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: '<html>not the API</html>',
    }
  }
  login(1)
  let container = await mount()
  assert.match(container.textContent ?? '', /administrator session is required/)
  assert.equal(calls, 0)
  await dispose?.()
  dispose = undefined
  document.body.replaceChildren()
  login(100)
  container = await mount()
  await until(
    () => container.textContent?.includes('Oops! Something went wrong') === true
  )
  assert.equal(calls, 1)
  assert.match(
    container.textContent ?? '',
    /The plugin response does not match this release/
  )
  assert.doesNotMatch(container.textContent ?? '', /No data available/)
})

test('failed refresh retains the last registry as stale and distinguishes an upstream 503', async () => {
  login(100)
  let calls = 0
  api.defaults.adapter = async (config) => {
    if (++calls > 1) {
      throw new AxiosError(
        'unavailable',
        'ERR_BAD_RESPONSE',
        config,
        undefined,
        {
          config,
          headers: {},
          status: 503,
          statusText: 'Unavailable',
          data: { success: false, message: 'synthetic outage' },
        }
      )
    }
    return {
      config,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: {
        success: true,
        data: [
          {
            name: 'toolmarket',
            version: '1.0.0',
            api_version: 1,
            registration_sha256: 'b'.repeat(64),
            available: true,
            installed: true,
            enabled: true,
          },
        ],
      },
    }
  }
  const container = await mount()
  await until(() => container.textContent?.includes('toolmarket') === true)
  assert.match(container.textContent ?? '', /Registration fingerprint/)
  assert.match(container.textContent ?? '', /Route configuration/)
  await act(async () => container.querySelector('button')?.click())
  await until(
    () =>
      container.textContent?.includes('Plugin status is not current') === true
  )
  assert.match(container.textContent ?? '', /toolmarket/)
  assert.match(
    container.textContent ?? '',
    /extension service is unavailable \(503\)/
  )
  assert.equal(calls, 2)
})

const plugin = {
  name: 'store',
  version: '1.0.0',
  api_version: 1,
  registration_sha256: 'c'.repeat(64),
  available: true,
  installed: true,
  enabled: true,
}
function button(label: string, root: ParentNode = document) {
  const found = [...root.querySelectorAll('button')].find(
    (element) => element.textContent === label
  )
  assert.ok(found, `Missing button: ${label}`)
  return found as HTMLButtonElement
}
function response(
  config: import('axios').InternalAxiosRequestConfig,
  data: unknown
) {
  return {
    config,
    headers: {},
    status: 200,
    statusText: 'OK',
    data: { success: true, data },
  }
}

test('confirmed disable sends one scoped write and reads back the actual registry', async () => {
  login(100)
  let state = { ...plugin }
  const calls: string[] = []
  api.defaults.adapter = async (config) => {
    calls.push(`${config.method} ${config.url}`)
    if (config.method === 'post') {
      assert.equal(config.url, '/api/extensions/plugins/store/disable')
      assert.equal(config.data, undefined)
      assert.deepEqual(config.authScope, {
        userId: 901,
        sessionId: 'synthetic-session',
      })
      state = { ...state, enabled: false }
      return response(config, state)
    }
    return response(config, [state])
  }
  const container = await mount()
  await until(() => container.textContent?.includes('Plugin details') === true)
  await act(async () => button('Disable', container).click())
  await until(() => document.querySelector('[role="alertdialog"]') !== null)
  assert.equal(calls.length, 1, 'opening confirmation must not mutate')
  await act(async () => button('Confirm').click())
  await until(
    () =>
      calls.length === 3 &&
      container.textContent?.includes('The plugin change was applied.') === true
  )
  assert.deepEqual(calls, [
    'get /api/extensions/plugins',
    'post /api/extensions/plugins/store/disable',
    'get /api/extensions/plugins',
  ])
  assert.ok(button('Enable', container))
})

test('a lost write response is recovered by a read without automatically repeating the write', async () => {
  login(100)
  let state = { ...plugin }
  let writes = 0
  let reads = 0
  api.defaults.adapter = async (config) => {
    if (config.method === 'post') {
      writes++
      state = { ...state, enabled: false }
      throw new AxiosError('connection interrupted', 'ERR_NETWORK', config)
    }
    reads++
    return response(config, [state])
  }
  const container = await mount()
  await until(() => container.textContent?.includes('Plugin details') === true)
  await act(async () => button('Disable', container).click())
  await until(() => document.querySelector('[role="alertdialog"]') !== null)
  await act(async () => button('Confirm').click())
  await until(
    () =>
      reads === 2 &&
      container.textContent?.includes('The change could not be confirmed.') ===
        true
  )
  assert.equal(writes, 1)
  assert.equal(button('Enable', container).disabled, false)
  assert.doesNotMatch(
    container.textContent ?? '',
    /The plugin change was applied/
  )
})

test('dependencies prevent disabling an active requirement and unconfigured modules cannot be installed', async () => {
  login(100)
  api.defaults.adapter = async (config) =>
    response(config, [
      plugin,
      { ...plugin, name: 'support', requires: ['store'] },
      {
        ...plugin,
        name: 'hero-sms',
        available: false,
        installed: false,
        enabled: false,
      },
    ])
  const container = await mount()
  await until(() => container.textContent?.includes('support') === true)
  const store = container.querySelector('[data-plugin="store"]')
  assert.ok(store)
  const support = container.querySelector('[data-plugin="support"]')
  assert.ok(support)
  const unconfigured = container.querySelector('[data-plugin="hero-sms"]')
  assert.ok(unconfigured)
  assert.equal(
    store.querySelector('a')?.getAttribute('href'),
    '/extensions/plugins?configure=store'
  )
  assert.equal(button('Disable', store).disabled, true)
  assert.equal(button('Disable', support).disabled, false)
  assert.equal(unconfigured.querySelector('button'), null)
  assert.equal(unconfigured.querySelector('a'), null)
  assert.match(unconfigured.textContent ?? '', /Setup required/)
  assert.match(unconfigured.textContent ?? '', /hero_sms.core_token_file/)
  assert.match(unconfigured.textContent ?? '', /host.modules/)
  assert.match(
    store.textContent ?? '',
    /Disable dependent modules first: support/
  )
})

test('installation binds the advertised manifest and every lifecycle response is validated', async () => {
  login(100)
  const scope = readNativeUsersScope()
  assert.ok(scope)
  const requests: string[] = []
  api.defaults.adapter = async (config) => {
    requests.push(`${config.method} ${config.url}`)
    if (config.url?.endsWith('/install')) {
      assert.deepEqual(JSON.parse(config.data), {
        name: plugin.name,
        version: plugin.version,
        api_version: plugin.api_version,
        registration_sha256: plugin.registration_sha256,
      })
      return response(config, { ...plugin, enabled: false })
    }
    if (config.url?.endsWith('/reload')) {
      assert.equal(config.data, undefined)
      return response(config, null)
    }
    if (config.method === 'delete') {
      return response(config, { ...plugin, installed: false, enabled: false })
    }
    return response(config, { ...plugin, name: 'unexpected-plugin' })
  }
  await changePlugin(
    scope,
    { ...plugin, installed: false, enabled: false },
    'install'
  )
  await assert.rejects(
    changePlugin(scope, { ...plugin, enabled: false }, 'enable')
  )
  await changePlugin(scope, { ...plugin, enabled: false }, 'uninstall')
  await reloadPlugins(scope)
  assert.equal(requests.length, 4)
  assert.equal(requests[2], 'delete /api/extensions/plugins/store')
})

test('plugin search matches its purpose and unavailable filter keeps lifecycle guards', async () => {
  login(100)
  api.defaults.adapter = async (config) =>
    response(config, [
      plugin,
      {
        ...plugin,
        name: 'checkins',
        available: false,
        installed: false,
        enabled: false,
      },
    ])
  const container = await mount()
  await until(() => container.querySelectorAll('[data-plugin]').length === 2)
  const input = container.querySelector('input')
  assert.ok(input)
  const setter = Object.getOwnPropertyDescriptor(
    window.HTMLInputElement.prototype,
    'value'
  )?.set
  assert.ok(setter)
  await act(async () => {
    setter.call(input, 'purchases')
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
  await until(() => container.querySelectorAll('[data-plugin]').length === 1)
  assert.ok(container.querySelector('[data-plugin="store"]'))
  await act(async () => {
    setter.call(input, '')
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
  const select = container.querySelector('select')
  assert.ok(select)
  await act(async () => {
    select.value = 'unavailable'
    select.dispatchEvent(new Event('change', { bubbles: true }))
  })
  await until(() => container.querySelectorAll('[data-plugin]').length === 1)
  const unconfigured = container.querySelector('[data-plugin="checkins"]')
  assert.ok(unconfigured)
  assert.equal(unconfigured.querySelector('button'), null)
  assert.equal(unconfigured.querySelector('a'), null)
})

test('switching sessions prevents a lifecycle write and discards a previous session response', async () => {
  login(100)
  const scope = readNativeUsersScope()
  assert.ok(scope)
  let calls = 0
  api.defaults.adapter = async (config) => {
    calls++
    login(100, 'another-session')
    return response(config, { ...plugin, enabled: false })
  }
  await assert.rejects(
    changePlugin(scope, plugin, 'disable'),
    /Session changed|Authentication changed/
  )
  assert.equal(calls, 1)
  await assert.rejects(
    changePlugin(scope, plugin, 'disable'),
    /Session changed|Authentication changed/
  )
  assert.equal(calls, 1)
})

test('plugin configuration saves through the checkins service with its observed revision', async () => {
  login(100)
  let writes = 0
  const settings = {
    revision: 7,
    enabled: false,
    min_quota: '1',
    max_quota: '2',
    writable: true,
    module_enabled: false,
    funding_available: true,
    funding_limit: '10',
  }
  api.defaults.adapter = async (config) => {
    if (config.url === '/api/extensions/plugins') {
      return response(config, [{ ...plugin, name: 'checkins', enabled: false }])
    }
    assert.equal(config.url, '/api/option/checkin')
    if (config.method === 'put') {
      const input = JSON.parse(config.data)
      assert.equal(input.revision, 7)
      assert.equal(input.enabled, true)
      assert.ok(input.operation_key)
      writes++
      return response(config, { ...settings, enabled: true, revision: 8 })
    }
    return response(config, settings)
  }
  const container = await mount('checkins')
  await until(() => !!container.querySelector('#checkin-enabled'))
  await act(async () =>
    container.querySelector<HTMLElement>('#checkin-enabled')?.click()
  )
  const form = container.querySelector('form')
  assert.ok(form)
  await act(async () => {
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
  })
  await until(
    () => writes === 1 && button('Save check-in settings', container).disabled
  )
})

test('plugin configuration retains owner permission and excludes Core-owned HeroSMS settings', async () => {
  login(10)
  api.defaults.adapter = async (config) => {
    assert.equal(config.url, '/api/extensions/plugins')
    return response(config, [
      { ...plugin, name: 'checkins' },
      { ...plugin, name: 'hero-sms' },
    ])
  }
  const container = await mount('checkins')
  await until(
    () => container.textContent?.includes('Owner access required') === true
  )
  await dispose?.()
  const inventory = await mount()
  await until(() => !!inventory.querySelector('[data-plugin="hero-sms"]'))
  assert.equal(inventory.querySelector('[data-plugin="hero-sms"] a'), null)
})

test('community navigation submits the observed value and preserves extension fields', async () => {
  login(100)
  const original = '{ "home": true, "custom": "retained" }'
  let writes = 0
  api.defaults.adapter = async (config) => {
    if (config.url === '/api/extensions/plugins') {
      return response(config, [{ ...plugin, name: 'community' }])
    }
    assert.equal(config.url, '/api/site/navigation')
    let value = original
    if (config.method === 'put') {
      const input = JSON.parse(config.data)
      assert.equal(input.expected_value, original)
      value = input.value
      assert.equal(JSON.parse(value).home, false)
      assert.equal(JSON.parse(value).custom, 'retained')
      writes++
    }
    return {
      ...response(config, { value, revision: writes + 1 }),
      data: {
        success: true,
        message: '',
        data: { value, revision: writes + 1 },
      },
    }
  }
  const container = await mount('community')
  await until(() => !!container.querySelector('[role="switch"]'))
  const control = container.querySelector<HTMLElement>('[role="switch"]')
  assert.ok(control)
  await act(async () => control.click())
  await act(async () => button('Save navigation', container).click())
  await until(() => writes === 1)
})

test('community sidebar saves the observed sidebar independently and reads it on remount', async () => {
  login(100)
  const header = '{ "home": false }'
  let sidebar = '{ "chat": { "enabled": "0" } }'
  let revision = 7
  api.defaults.adapter = async (config) => {
    if (config.url === '/api/extensions/plugins') {
      return response(config, [{ ...plugin, name: 'community' }])
    }
    assert.equal(config.url, '/api/site/navigation')
    let data
    if (config.method === 'put') {
      const input = JSON.parse(config.data)
      assert.equal(input.key, 'SidebarModulesAdmin')
      assert.equal(input.expected_value, sidebar)
      assert.equal(JSON.parse(input.value).chat.enabled, true)
      sidebar = input.value
      data = { value: sidebar, revision: ++revision }
    } else {
      data = {
        value: header,
        revision: 5,
        sidebar: { value: sidebar, revision },
      }
    }
    return {
      ...response(config, data),
      data: { success: true, message: '', data },
    }
  }
  const container = await mount('community', 'sidebar')
  await until(() => !!container.querySelector('[role="switch"]'))
  const control = container.querySelector<HTMLElement>('[role="switch"]')!
  assert.equal(control.getAttribute('aria-checked'), 'false')
  await act(async () => control.click())
  await act(async () => button('Save sidebar modules', container).click())
  await until(() => revision === 8)
  await dispose?.()
  const refreshed = await mount('community', 'sidebar')
  await until(
    () =>
      refreshed
        .querySelector('[role="switch"]')
        ?.getAttribute('aria-checked') === 'true'
  )
})

test('a header-only community server cannot overwrite site sidebar defaults', async () => {
  login(100)
  api.defaults.adapter = async (config) => {
    if (config.url === '/api/extensions/plugins')
      return response(config, [{ ...plugin, name: 'community' }])
    assert.equal(config.method, 'get')
    assert.equal(config.url, '/api/site/navigation')
    return {
      ...response(config, {}),
      data: { success: true, message: '', data: { value: '{}', revision: 1 } },
    }
  }
  const container = await mount('community', 'sidebar')
  await until(
    () =>
      container.textContent?.includes(
        'This server does not expose controls for this section.'
      ) === true
  )
  assert.equal(container.querySelector('[role="switch"]'), null)
})
