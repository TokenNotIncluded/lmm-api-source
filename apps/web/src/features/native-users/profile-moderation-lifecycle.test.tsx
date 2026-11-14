/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, beforeEach, test } from 'node:test'

import { Window } from 'happy-dom'

const unexpected: string[] = []
const reject = ({ request }: { request: { url: string } }): never => {
  unexpected.push(request.url)
  throw new Error(`Uncontrolled network: ${request.url}`)
}
const dom = new Window({
  url: 'https://console.example.test/settings',
  settings: {
    fetch: {
      interceptor: {
        beforeAsyncRequest: async (c) => reject(c),
        beforeSyncRequest: reject,
      },
    },
  },
})
dom.document.write('<!doctype html><html><head></head><body></body></html>')
Object.defineProperty(dom.document, 'compatMode', {
  configurable: true,
  value: 'CSS1Compat',
})
const originalGlobals = new Map<string, PropertyDescriptor | undefined>()
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLInputElement',
  'HTMLTextAreaElement',
  'HTMLButtonElement',
  'HTMLLabelElement',
  'SVGElement',
  'Node',
  'NodeFilter',
  'Element',
  'Event',
  'MouseEvent',
  'PointerEvent',
  'FocusEvent',
  'CustomEvent',
  'MutationObserver',
  'ResizeObserver',
  'requestAnimationFrame',
  'cancelAnimationFrame',
  'getComputedStyle',
  'localStorage',
  'sessionStorage',
  'matchMedia',
  'customElements',
] as const) {
  originalGlobals.set(key, Object.getOwnPropertyDescriptor(globalThis, key))
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: dom[key],
  })
}
originalGlobals.set(
  'IS_REACT_ACT_ENVIRONMENT',
  Object.getOwnPropertyDescriptor(globalThis, 'IS_REACT_ACT_ENVIRONMENT')
)
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
const React = await import('react')
const { act } = React
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider, notifyManager } =
  await import('@tanstack/react-query')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { SettingsPageProvider } =
  await import('@/features/system-settings/components/settings-page-context')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { toast } = await import('sonner')
const messages: { kind: string; message: string }[] = []
const originalToasts = new Map(
  ['error', 'success', 'info', 'warning'].map((kind) => [
    kind,
    Reflect.get(toast, kind),
  ])
)
for (const kind of ['error', 'success', 'info', 'warning'] as const) {
  Reflect.set(toast, kind, (message: unknown) => {
    messages.push({ kind, message: String(message) })
    return 0
  })
}
const i18n = createInstance()
await i18n.use(initReactI18next).init({
  lng: 'en',
  fallbackLng: 'en',
  resources: { en: { translation: {} }, fr: { translation: {} } },
})
function actor(id = 41, sid = 'session-A', role = 100) {
  useAuthStore.getState().auth.setBundle({
    access_token: `synthetic-${id}-${sid}`,
    token_type: 'Bearer',
    access_expires_at: 2000000000,
    user: { id, username: `synthetic-${id}`, role },
    session: {
      sid,
      current: true,
      login_method: 'password',
      ip: '',
      user_agent: '',
      created_at: 1,
      last_active_at: 1,
      expires_at: 2000000000,
    },
  })
}
function deferred<T = unknown>() {
  let resolve!: (v: T) => void, reject!: (e: Error) => void
  const promise = new Promise<T>((yes, no) => {
    resolve = yes
    reject = no
  })
  return { promise, resolve, reject }
}
async function flush() {
  await act(async () => {
    await new Promise<void>((resolve) => notifyManager.schedule(resolve))
  })
}
async function until(check: () => boolean, label: string) {
  for (let i = 0; i < 80 && !check(); i++) await flush()
  assert.ok(check(), `${label}; DOM=${document.body.textContent?.slice(-1800)}`)
}
const mounts: {
  root: ReturnType<typeof createRoot>
  client: InstanceType<typeof QueryClient>
}[] = []
async function mount(element: React.ReactNode) {
  const host = document.createElement('div'),
    actions = document.createElement('div')
  document.body.append(host, actions)
  const root = createRoot(host)
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: Infinity },
      mutations: { retry: false },
    },
  })
  client.setQueryData(['system-options'], {
    success: true,
    message: '',
    data: [],
  })
  mounts.push({ root, client })
  const render = async (node: React.ReactNode) => {
    await act(async () =>
      root.render(
        <QueryClientProvider client={client}>
          <I18nextProvider i18n={i18n}>
            <SettingsPageProvider actionsContainer={actions}>
              {node}
            </SettingsPageProvider>
          </I18nextProvider>
        </QueryClientProvider>
      )
    )
    await flush()
  }
  await render(element)
  return { host, actions, client, render }
}
function button(text: string, scope: ParentNode = document) {
  const found = [...scope.querySelectorAll<HTMLButtonElement>('button')].find(
    (b) => b.textContent?.trim() === text
  )
  assert.ok(found, `button missing: ${text}`)
  return found
}
async function click(element: HTMLElement) {
  await act(async () => element.click())
  await flush()
}
async function edit(
  input: HTMLInputElement | HTMLTextAreaElement,
  value: string
) {
  assert.ok(input)
  const proto =
    input.tagName === 'TEXTAREA'
      ? dom.HTMLTextAreaElement.prototype
      : dom.HTMLInputElement.prototype
  const setter = Object.getOwnPropertyDescriptor(proto, 'value')?.set
  assert.ok(setter)
  await act(async () => {
    setter.call(input, value)
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
  await flush()
}
afterEach(async () => {
  for (const { root, client } of mounts.splice(0)) {
    await act(async () => root.unmount())
    client.clear()
  }
  document.body.replaceChildren()
  assert.deepEqual(unexpected.splice(0), [])
  messages.splice(0)
})
after(() => {
  for (const [kind, method] of originalToasts) Reflect.set(toast, kind, method)
  dom.close()
  for (const [key, descriptor] of originalGlobals) {
    if (descriptor) Object.defineProperty(globalThis, key, descriptor)
    else Reflect.deleteProperty(globalThis, key)
  }
})

const { NativeDisplayNameEditor } = await import('./profile-editor')
const { readNativeUsersScope } = await import('./access')
const { PROFILE_PROTOCOL } = await import('./profile-contract')
const { ModerationSettingsSection } =
  await import('@/features/system-settings/security/moderation-settings-section')
const { installSupportedAuthBrowser } =
  await import('@/test-support/supported-auth-browser')
installSupportedAuthBrowser()
const originalAdapter = api.defaults.adapter
type RequestRecord = {
  method: string
  path: string
  body: Record<string, unknown>
  scope?: { userId: number | undefined; sessionId: string | undefined }
}
let requests: RequestRecord[]
let transport: (request: RequestRecord) => Promise<unknown>
const capabilities = {
  version: 1,
  contract: PROFILE_PROTOCOL,
  operations: { read: true, update_display_name: true },
  target_platform_roles: ['user'],
  display_name_limits: {
    max_code_points: 20,
    max_utf8_bytes: 80,
    controls_allowed: false,
  },
  preconditions: ['auth_version', 'display_name'],
}
const profile = {
  version: 1,
  contract: PROFILE_PROTOCOL,
  user_id: '9007199254740993',
  auth_version: '9007199254740995',
  display_name: 'server name',
  platform_role: 'user',
  active: false,
  account_active: false,
}
const capabilityPath = '/core/v1/admin/user-profiles/capabilities'
const profilePath = `/core/v1/admin/user-profiles/${profile.user_id}`
const defaults = {
  ModerationEnabled: false,
  ModerationPolicyScope: 'account_group' as const,
  ModerationSafetyIdentifierEnabled: false,
  ModerationGroup: 'default',
  ModerationModel: 'omni-moderation-latest' as const,
  ModerationGroupPolicies: '{}',
}
beforeEach(() => {
  actor()
  requests = []
  transport = async (request) => {
    if (request.method === 'get') {
      if (request.path === capabilityPath) return capabilities
      if (request.path.startsWith('/core/v1/admin/user-profiles/')) {
        return { ...profile, user_id: request.path.split('/').at(-1) }
      }
      if (request.path === '/api/group/') {
        return { success: true, data: ['default', 'route-one'] }
      }
      if (request.path === '/api/security/admin/moderation/models') {
        return {
          success: true,
          data: { group: 'default', models: ['omni-moderation-latest'] },
        }
      }
      if (request.path === '/api/security/admin/moderation-reviews') {
        return {
          success: true,
          data: { rows: [], total: 0, page: 1, page_size: 20 },
        }
      }
      if (request.path === '/api/security/admin/violation-fee-appeals') {
        return { success: true, data: [] }
      }
      if (request.path === '/api/security/admin/moderation-stats') {
        return {
          success: true,
          data: {
            pending: 0,
            running: 0,
            completed: 0,
            failed: 0,
            cancelled: 0,
            flagged: 0,
            fined: 0,
            charged_quota: 0,
            settings: { enabled: false },
          },
        }
      }
    }
    if (
      request.method === 'patch' &&
      request.path.startsWith('/core/v1/admin/user-profiles/')
    ) {
      return {
        version: 1,
        contract: PROFILE_PROTOCOL,
        changed: true,
        profile: {
          ...profile,
          user_id: request.path.split('/').at(-1),
          display_name: request.body.display_name,
        },
      }
    }
    if (request.method === 'post' && request.path === '/api/option/bulk') {
      return { success: true, message: '' }
    }
    unexpected.push(`${request.method} ${request.path}`)
    throw new Error('Unexpected transport')
  }
  api.defaults.adapter = async (config) => {
    const request = {
      method: config.method ?? '',
      path: config.url ?? '',
      body:
        typeof config.data === 'string'
          ? JSON.parse(config.data)
          : (config.data ?? {}),
      scope: Reflect.get(config, 'authScope'),
    }
    requests.push(request)
    try {
      return {
        config,
        data: await transport(request),
        status: 200,
        statusText: 'fixture',
        headers: {},
      }
    } catch (error) {
      if (error instanceof Error) Object.assign(error, { config })
      throw error
    }
  }
})
afterEach(() => {
  api.defaults.adapter = originalAdapter
})
const writes = () => requests.filter(({ method }) => method !== 'get')
function profileElement(
  userId = profile.user_id,
  onUpdated = () => {},
  onAccessDenied = () => {}
) {
  const scope = readNativeUsersScope()
  assert.ok(scope)
  return (
    <NativeDisplayNameEditor
      scope={scope}
      userId={userId}
      onUpdated={onUpdated}
      onAccessDenied={onAccessDenied}
    />
  )
}
function nameInput(userId = profile.user_id) {
  const node = document.getElementById(
    `native-display-name-${userId}`
  ) as HTMLInputElement | null
  assert.ok(node)
  return node
}
async function openProfile() {
  await click(button('Edit · Display name'))
}
function switches(host: HTMLElement) {
  return [...host.querySelectorAll<HTMLElement>('[role="switch"]')]
}
async function requestGroup(host: HTMLElement) {
  const trigger = [
    ...host.querySelectorAll<HTMLElement>('[role="combobox"]'),
  ].find((node) => node.textContent?.trim() === 'Account group')
  assert.ok(trigger)
  await click(trigger)
  const option = [
    ...document.querySelectorAll<HTMLElement>('[role="option"]'),
  ].find((node) => node.textContent === 'Request group')
  assert.ok(option)
  await click(option)
}
async function submit(ui: Awaited<ReturnType<typeof mount>>) {
  await click(button('Save moderation settings', ui.actions))
}

test('profile target switch closes the old draft and ignores a late read without issuing writes', async () => {
  const pending = deferred(),
    base = transport,
    other = '9007199254740994'
  transport = async (request) =>
    request.method === 'get' && request.path === profilePath
      ? pending.promise
      : base(request)
  const ui = await mount(profileElement())
  await openProfile()
  await until(
    () => requests.some(({ path }) => path === profilePath),
    'old read pending'
  )
  await ui.render(profileElement(other))
  assert.equal(document.querySelector('[role="dialog"]'), null)
  await act(async () => pending.resolve(profile))
  await flush()
  assert.doesNotMatch(document.body.textContent ?? '', /server name/)
  await openProfile()
  await until(
    () => nameInput(other).value === profile.display_name,
    'new target loaded'
  )
  assert.deepEqual(
    requests.map(({ path }) => path),
    [
      capabilityPath,
      profilePath,
      capabilityPath,
      `/core/v1/admin/user-profiles/${other}`,
    ]
  )
  assert.equal(writes().length, 0)
  assert.ok(
    requests.every(
      ({ scope }) => scope?.userId === 41 && scope.sessionId === 'session-A'
    )
  )
})

test('profile cancel and reopen establishes a new draft and a late save cannot close it or report success', async () => {
  const pending = deferred(),
    base = transport
  transport = async (request) =>
    request.method === 'patch' ? pending.promise : base(request)
  let updated = 0
  await mount(profileElement(profile.user_id, () => updated++))
  await openProfile()
  await until(
    () => nameInput().value === profile.display_name,
    'profile loaded'
  )
  await edit(nameInput(), 'old submitted draft')
  await click(button('Save changes'))
  await until(() => writes().length === 1, 'save pending')
  await click(button('Cancel'))
  await openProfile()
  await until(
    () => nameInput().value === profile.display_name,
    'reopened profile'
  )
  await edit(nameInput(), 'new unsaved draft')
  await act(async () =>
    pending.resolve({
      version: 1,
      contract: PROFILE_PROTOCOL,
      changed: true,
      profile: { ...profile, display_name: 'old submitted draft' },
    })
  )
  await flush()
  assert.equal(nameInput().value, 'new unsaved draft')
  assert.equal(updated, 0)
  assert.equal(messages.filter(({ kind }) => kind === 'success').length, 0)
  assert.deepEqual(writes()[0].body, {
    expected_auth_version: profile.auth_version,
    expected_display_name: profile.display_name,
    display_name: 'old submitted draft',
  })
})

test('profile callback changes during a write use the latest committed callback without restarting reads', async () => {
  const pending = deferred(),
    base = transport
  transport = async (request) =>
    request.method === 'patch' ? pending.promise : base(request)
  let oldUpdated = 0,
    newUpdated = 0
  const ui = await mount(profileElement(profile.user_id, () => oldUpdated++))
  await openProfile()
  await until(
    () => nameInput().value === profile.display_name,
    'profile loaded'
  )
  await edit(nameInput(), 'submitted name')
  await click(button('Save changes'))
  await ui.render(profileElement(profile.user_id, () => newUpdated++))
  await act(async () =>
    pending.resolve({
      version: 1,
      contract: PROFILE_PROTOCOL,
      changed: true,
      profile: { ...profile, display_name: 'submitted name' },
    })
  )
  await until(
    () => document.querySelector('[role="dialog"]') === null,
    'successful close'
  )
  assert.equal(oldUpdated, 0)
  assert.equal(newUpdated, 1)
  assert.deepEqual(
    requests.map(({ path }) => path),
    [capabilityPath, profilePath, profilePath]
  )
})

test('profile actual capability denial never reads the target or dispatches PATCH/PUT', async () => {
  transport = async () => ({
    ...capabilities,
    operations: { read: true, update_display_name: false },
  })
  await mount(profileElement())
  await openProfile()
  await until(
    () =>
      document.body.textContent?.includes('Failed to load profile') === true,
    'capability denial'
  )
  assert.equal(button('Save changes').disabled, true)
  assert.deepEqual(
    requests.map(({ path }) => path),
    [capabilityPath]
  )
  assert.equal(writes().length, 0)
})

test('moderation source refresh retains edited fields, updates untouched fields and reset uses the latest baseline', async () => {
  const ui = await mount(<ModerationSettingsSection defaultValues={defaults} />)
  await requestGroup(ui.host)
  const incoming = {
    ...defaults,
    ModerationGroup: 'route-one',
    ModerationSafetyIdentifierEnabled: true,
  }
  await ui.render(<ModerationSettingsSection defaultValues={incoming} />)
  assert.ok(
    [...ui.host.querySelectorAll('[role="combobox"]')].some(
      (node) => node.textContent?.trim() === 'Request group'
    )
  )
  assert.equal(switches(ui.host)[1].getAttribute('aria-checked'), 'true')
  await click(button('Reset', ui.host))
  assert.ok(
    [...ui.host.querySelectorAll('[role="combobox"]')].some(
      (node) => node.textContent?.trim() === 'Account group'
    )
  )
  assert.equal(switches(ui.host)[1].getAttribute('aria-checked'), 'true')
  assert.equal(button('Save moderation settings', ui.actions).disabled, true)
  assert.equal(writes().length, 0)
})

test('moderation real read-only capabilities reject edits with no POST or PUT and retain the draft', async () => {
  const ui = await mount(<ModerationSettingsSection defaultValues={defaults} />)
  await act(async () =>
    ui.client.setQueryData(['system-options'], {
      success: true,
      message: '',
      data: [],
      capabilities: { read_only: true, writable_keys: [] },
    })
  )
  await requestGroup(ui.host)
  await submit(ui)
  await until(
    () =>
      messages.some(
        ({ kind, message }) => kind === 'error' && message === 'Read only'
      ),
    'real permission refusal'
  )
  assert.equal(writes().length, 0)
  assert.equal(button('Reset', ui.host).disabled, false)
  assert.ok(
    [...ui.host.querySelectorAll('[role="combobox"]')].some(
      (node) => node.textContent?.trim() === 'Request group'
    )
  )
})

test('moderation failed legacy write preserves the draft and reset remains read-only', async () => {
  const base = transport
  transport = async (request) =>
    request.method === 'post'
      ? { success: false, message: 'synthetic moderation rejection' }
      : base(request)
  const ui = await mount(<ModerationSettingsSection defaultValues={defaults} />)
  await requestGroup(ui.host)
  await submit(ui)
  await until(
    () =>
      messages.some(
        ({ message }) => message === 'synthetic moderation rejection'
      ),
    'write rejection'
  )
  assert.deepEqual(
    writes().map(({ method, path, body }) => ({ method, path, body })),
    [
      {
        method: 'post',
        path: '/api/option/bulk',
        body: { values: { ModerationPolicyScope: 'request_group' } },
      },
    ]
  )
  assert.equal(button('Reset', ui.host).disabled, false)
  await click(button('Reset', ui.host))
  assert.equal(button('Save moderation settings', ui.actions).disabled, true)
  assert.equal(writes().length, 1)
})

test('moderation late acknowledgement cannot overwrite a new session draft', async () => {
  const pending = deferred(),
    base = transport
  transport = async (request) =>
    request.method === 'post' ? pending.promise : base(request)
  const ui = await mount(<ModerationSettingsSection defaultValues={defaults} />)
  await requestGroup(ui.host)
  await submit(ui)
  await until(() => writes().length === 1, 'old session save pending')
  await act(async () => actor(41, 'session-B'))
  assert.ok(
    [...ui.host.querySelectorAll('[role="combobox"]')].some(
      (node) => node.textContent?.trim() === 'Account group'
    )
  )
  await click(switches(ui.host)[0])
  await act(async () => pending.resolve({ success: true, message: '' }))
  await flush()
  assert.equal(switches(ui.host)[0].getAttribute('aria-checked'), 'true')
  assert.equal(button('Reset', ui.host).disabled, false)
  assert.equal(writes().length, 1)
})

test('moderation acknowledgement retains newer edits and reset follows a source refresh during the write', async () => {
  const pending = deferred(),
    base = transport
  transport = async (request) =>
    request.method === 'post' ? pending.promise : base(request)
  const ui = await mount(<ModerationSettingsSection defaultValues={defaults} />)
  await requestGroup(ui.host)
  await submit(ui)
  await until(() => writes().length === 1, 'save pending')
  const incoming = { ...defaults, ModerationGroup: 'route-one' }
  await ui.render(<ModerationSettingsSection defaultValues={incoming} />)
  await click(switches(ui.host)[0])
  await act(async () => pending.resolve({ success: true, message: '' }))
  await until(
    () => !button('Reset', ui.host).disabled,
    'newer edit remains pending'
  )
  assert.equal(switches(ui.host)[0].getAttribute('aria-checked'), 'true')
  assert.deepEqual(
    writes().map(({ body }) => body),
    [{ values: { ModerationPolicyScope: 'request_group' } }]
  )
  await click(button('Reset', ui.host))
  assert.equal(switches(ui.host)[0].getAttribute('aria-checked'), 'false')
  assert.ok(
    [...ui.host.querySelectorAll('[role="combobox"]')].some(
      (node) => node.textContent?.trim() === 'Account group'
    )
  )
  assert.ok(
    [...ui.host.querySelectorAll('[role="combobox"]')].some(
      (node) => node.textContent?.trim() === 'route-one'
    )
  )
  assert.equal(button('Save moderation settings', ui.actions).disabled, true)
  assert.equal(writes().length, 1)
})
