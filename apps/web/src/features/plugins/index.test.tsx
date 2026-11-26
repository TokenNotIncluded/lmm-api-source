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
  scrollTo: dom.scrollTo.bind(dom),
  getComputedStyle: dom.getComputedStyle.bind(dom),
  requestAnimationFrame: dom.requestAnimationFrame.bind(dom),
  cancelAnimationFrame: dom.cancelAnimationFrame.bind(dom),
})) {
  Object.defineProperty(globalThis, name, { configurable: true, value })
}
const { act, useState } = await import('react')
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

async function mount(panel?: string) {
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const { createMemoryHistory, createRootRoute, createRouter, RouterProvider } =
    await import('@tanstack/react-router')
  const routeTree = createRootRoute({
    component: function TestPluginRoute() {
      const [active, setActive] = useState(panel)
      return (
        <PluginsPage
          panel={active}
          onPanelChange={panel ? setActive : undefined}
        />
      )
    },
  })
  const router = createRouter({
    routeTree,
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
    ['Refresh', 'Reload routes', 'Disable', 'Configure']
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
  assert.equal(button('Disable', store).disabled, true)
  assert.equal(button('Disable', support).disabled, false)
  assert.equal(unconfigured.querySelector('a'), null)
  assert.equal(
    button('Configure', unconfigured).getAttribute('aria-expanded'),
    'false'
  )
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
  assert.equal(unconfigured.querySelector('a'), null)
  assert.equal(
    button('Configure', unconfigured).getAttribute('aria-expanded'),
    'false'
  )
})

test('configuration expands inside the matching plugin card', async () => {
  login(100)
  api.defaults.adapter = async (config) =>
    response(config, [{ ...plugin, name: 'assistant' }])
  const container = await mount()
  await until(() =>
    Boolean(container.querySelector('[data-plugin="assistant"]'))
  )
  const card = container.querySelector('[data-plugin="assistant"]')
  assert.ok(card)
  const configure = button('Configure', card)
  assert.equal(configure.tagName, 'BUTTON')
  await act(async () => configure.click())
  assert.equal(configure.getAttribute('aria-expanded'), 'true')
  assert.ok(card.querySelector('#plugin-config-assistant'))
  await act(async () => configure.click())
  assert.equal(configure.getAttribute('aria-expanded'), 'false')
  assert.equal(card.querySelector('#plugin-config-assistant'), null)
})

test('manual order submits a complete dependency-safe order and auto sort reads it back', async () => {
  login(100)
  const details = new Map([
    ['identity', { ...plugin, name: 'identity' }],
    ['store', { ...plugin, required_by: ['support'] }],
    ['support', { ...plugin, name: 'support', requires: ['store'] }],
    ['operations', { ...plugin, name: 'operations' }],
  ])
  let names = ['identity', 'store', 'support', 'operations']
  const writes: string[] = []
  api.defaults.adapter = async (config) => {
    if (config.method === 'put') {
      assert.equal(config.url, '/api/extensions/plugins/order')
      names = JSON.parse(config.data).order
      assert.deepEqual(names, ['identity', 'store', 'operations', 'support'])
      writes.push('manual')
    }
    if (config.method === 'post') {
      assert.equal(config.url, '/api/extensions/plugins/order/auto')
      names = ['identity', 'operations', 'store', 'support']
      writes.push('auto')
    }
    return response(
      config,
      names.map((name, index) => ({
        ...details.get(name),
        load_order: index + 1,
      }))
    )
  }
  const container = await mount()
  await until(() => Boolean(container.querySelector('#plugin-load-order')))
  const controls = (label: string) => {
    const group = container.querySelector(
      `[role="group"][aria-label="${label}"]`
    )
    assert.ok(group)
    return group
  }
  assert.equal(button('Move up', controls('Support')).disabled, true)
  await act(async () => button('Move up', controls('Operations')).click())
  await until(
    () =>
      writes.length === 1 && !button('Sort by dependencies', container).disabled
  )
  await act(async () => button('Sort by dependencies', container).click())
  await until(
    () =>
      writes.length === 2 && !button('Sort by dependencies', container).disabled
  )
  const first = container
    .querySelector('#plugin-load-order')
    ?.closest('section')
    ?.querySelector('li')
  assert.match(first?.textContent ?? '', /Operations/)
  assert.deepEqual(writes, ['manual', 'auto'])
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

test('RSS deep link reads the saved feeds and writes a revision in the operations card', async (context) => {
  login(100)
  const scrolled: string[] = []
  const original = HTMLElement.prototype.scrollIntoView
  HTMLElement.prototype.scrollIntoView = function (options) {
    if (typeof options === 'object' && options.block === 'start') {
      scrolled.push(this.id)
    }
  }
  context.after(() => {
    HTMLElement.prototype.scrollIntoView = original
  })
  let revision = 7
  let feeds = [
    {
      id: 'sample',
      name: 'Original feed',
      url: 'https://example.org/feed',
      enabled: true,
    },
  ]
  const writes: unknown[] = []
  api.defaults.adapter = async (config) => {
    if (config.url === '/api/extensions/plugins') {
      return response(config, [{ ...plugin, name: 'operations' }])
    }
    assert.equal(config.url, '/api/rss/settings')
    assert.deepEqual(config.authScope, {
      userId: 901,
      sessionId: 'synthetic-session',
    })
    if (config.method === 'put') {
      const body = JSON.parse(config.data)
      writes.push(body)
      assert.equal(config.skipBusinessError, true)
      assert.equal(config.skipErrorHandler, true)
      if (writes.length === 1) {
        return {
          ...response(config, {}),
          data: { success: false, message: 'Fixture save rejected' },
        }
      }
      revision += 1
      feeds = body.feeds
    }
    return response(config, { revision, feeds })
  }
  const container = await mount('rss')
  await until(() => container.textContent?.includes('Original feed') === true)
  const card = container.querySelector('[data-plugin="operations"]')
  assert.ok(card)
  assert.equal(document.activeElement?.id, 'plugin-config-operations')
  assert.deepEqual(scrolled, ['plugin-config-operations'])
  assert.equal(button('RSS', card).getAttribute('aria-pressed'), 'true')
  const toggle = card.querySelector<HTMLElement>('[role="switch"]')
  assert.ok(toggle)
  await act(async () => toggle.click())
  const save = button('Save feeds', card)
  await until(() => !save.disabled)
  await act(async () => {
    save.focus()
    save.click()
  })
  await until(
    () =>
      card
        .querySelector('[role="alert"]')
        ?.textContent?.includes('Save failed') === true
  )
  assert.equal(save.disabled, false)
  assert.equal(document.activeElement, card.querySelector('[role="alert"]'))
  await act(async () => {
    save.focus()
    save.click()
  })
  await until(
    () =>
      writes.length === 2 &&
      save.disabled &&
      card.querySelector('[role="status"]')?.textContent === 'Settings saved'
  )
  assert.equal(card.querySelector('[role="alert"]'), null)
  assert.equal(document.activeElement, card.querySelector('[role="status"]'))
  const expected = {
    revision: 7,
    feeds: [
      {
        id: 'sample',
        name: 'Original feed',
        url: 'https://example.org/feed',
        enabled: false,
      },
    ],
  }
  assert.deepEqual(writes, [expected, expected])
  await act(async () => button('Close', card).click())
  assert.equal(card.querySelector('#plugin-config-operations'), null)
  assert.equal(document.activeElement, button('Configure', card))
})

test('a standard administrator cannot mount a root-only configuration from the URL', async () => {
  login(10)
  api.defaults.adapter = async (config) => {
    assert.equal(config.url, '/api/extensions/plugins')
    return response(config, [{ ...plugin, name: 'store' }])
  }
  const container = await mount('store')
  await until(() => Boolean(container.querySelector('[data-plugin="store"]')))
  assert.equal(container.querySelector('#plugin-config-store'), null)
  assert.equal(
    [...container.querySelectorAll('button')].some(
      (element) => element.textContent === 'Configure'
    ),
    false
  )
})

test('community navigation validates the scoped value before acknowledging a write', async () => {
  login(100)
  let value = JSON.stringify({ home: true })
  const methods: string[] = []
  const proposed: string[] = []
  let approve = false
  api.defaults.adapter = async (config) => {
    if (config.url === '/api/extensions/plugins') {
      return response(config, [{ ...plugin, name: 'community' }])
    }
    assert.deepEqual(config.authScope, {
      userId: 901,
      sessionId: 'synthetic-session',
    })
    methods.push(config.method ?? '')
    if (config.url === '/api/site/navigation/validate') {
      assert.equal(config.method, 'post')
      proposed.push(JSON.parse(config.data).value)
      return {
        ...response(config, {}),
        data: { success: approve, message: approve ? '' : 'invalid' },
      }
    }
    assert.equal(config.url, '/api/site/navigation')
    if (config.method === 'put') {
      assert.equal(approve, true)
      value = JSON.parse(config.data).value
    }
    return {
      ...response(config, {}),
      data: { success: true, message: '', data: { value, revision: 1 } },
    }
  }
  const container = await mount('header-navigation')
  await until(() =>
    Boolean(container.querySelector('#plugin-config-community [role="switch"]'))
  )
  const card = container.querySelector('[data-plugin="community"]')
  assert.ok(card)
  const toggle = card.querySelector<HTMLElement>('[role="switch"]')
  assert.ok(toggle)
  await act(async () => toggle.click())
  await act(async () => button('Save navigation', card).click())
  await until(
    () => card.querySelector('[role="alert"]')?.textContent === 'Save failed'
  )
  assert.deepEqual(methods, ['get', 'post'])
  approve = true
  await act(async () => button('Save navigation', card).click())
  await until(() => methods.includes('put'))
  assert.deepEqual(
    methods.filter((x) => x !== 'get'),
    ['post', 'post', 'put']
  )
  assert.equal(proposed[0], proposed[1])
  assert.equal(proposed[1], value)
  assert.equal(JSON.parse(value).home, false)
})

test('opening a disabled operations plugin offers its existing enable action without requesting RSS', async () => {
  login(100)
  const reads: string[] = []
  api.defaults.adapter = async (config) => {
    reads.push(config.url ?? '')
    assert.equal(config.method, 'get')
    return response(config, [{ ...plugin, name: 'operations', enabled: false }])
  }
  const container = await mount('rss')
  await until(() =>
    Boolean(container.querySelector('#plugin-config-operations'))
  )
  const panel = container.querySelector('#plugin-config-operations')
  assert.ok(panel)
  assert.match(panel.textContent ?? '', /Enable this module/)
  await act(async () => button('Enable', panel).click())
  await until(() => Boolean(document.querySelector('[role="alertdialog"]')))
  assert.deepEqual(reads, ['/api/extensions/plugins'])
})

test('an installed disabled check-in plugin still opens its host-owned reward endpoint', async () => {
  login(100)
  const reads: string[] = []
  api.defaults.adapter = async (config) => {
    reads.push(config.url ?? '')
    assert.equal(config.method, 'get')
    return config.url === '/api/extensions/plugins'
      ? response(config, [{ ...plugin, name: 'checkins', enabled: false }])
      : response(config, {})
  }
  await mount('checkins')
  await until(() => reads.includes('/api/option/checkin'))
  assert.deepEqual(reads, ['/api/extensions/plugins', '/api/option/checkin'])
})
