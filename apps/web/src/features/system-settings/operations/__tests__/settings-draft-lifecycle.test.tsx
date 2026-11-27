/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import { after, afterEach, beforeEach, test } from 'node:test'

import { Window } from 'happy-dom'

const { mock } = createRequire(import.meta.url)('bun:test') as {
  mock: { module: (name: string, factory: () => unknown) => void }
}
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
const {
  QueryClient,
  QueryClientProvider,
  notifyManager,
  useMutation,
  useQueryClient,
} = await import('@tanstack/react-query')
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

mock.module(
  '@/features/system-settings/components/system-json-code-editor',
  () => ({
    SystemJsonCodeEditor: (props: {
      value: string
      onChange: (value: string) => void
      disabled: boolean
      configurationKey: string
    }) => (
      <textarea
        data-editor={props.configurationKey}
        value={props.value}
        disabled={props.disabled}
        onChange={(event) => props.onChange(event.target.value)}
      />
    ),
  })
)
mock.module(
  '@/features/system-settings/components/form-navigation-guard',
  () => ({
    FormNavigationGuard: ({ when }: { when: boolean }) => (
      <output data-dirty={String(when)} />
    ),
  })
)
mock.module('@/components/confirm-dialog', () => ({
  ConfirmDialog: (props: {
    open: boolean
    onOpenChange: (open: boolean) => void
    handleConfirm: () => void
  }) =>
    props.open ? (
      <div role='dialog'>
        <button type='button' onClick={() => props.onOpenChange(false)}>
          Cancel replacement
        </button>
        <button type='button' onClick={props.handleConfirm}>
          Confirm replacement
        </button>
      </div>
    ) : null,
}))
// Component-only write seam: live capabilities intentionally do not grant
// AntiRelay keys. Keep one test on the real guard; exercise form save lifecycle
// separately without claiming policy authorization or backend acceptance.
const realUseUpdateOption = (
  await import('@/features/system-settings/hooks/use-update-option')
).useUpdateOption
let componentWriteBoundary = false
mock.module('@/features/system-settings/hooks/use-update-option', () => ({
  useUpdateOption() {
    const guarded = realUseUpdateOption()
    const client = useQueryClient()
    const component = useMutation({
      retry: false,
      mutationFn: async (request: {
        key: string
        value: string | number | boolean
      }) => {
        const response = await api.put('/api/option/', request, {
          skipBusinessError: true,
          skipErrorHandler: true,
        })
        if (!response.data.success) throw new Error(response.data.message)
        return response.data
      },
      onSuccess: () =>
        client.invalidateQueries({ queryKey: ['system-options'] }),
    })
    return componentWriteBoundary ? component : guarded
  },
}))
const { RawJsonConfigurationSection } =
  await import('@/features/system-settings/operations/raw-json-configuration-section')
const { AntiRelaySection } =
  await import('@/features/system-settings/security/anti-relay-section')
const { useSystemOptions } =
  await import('@/features/system-settings/hooks/use-system-options')
const { installSupportedAuthBrowser } =
  await import('@/test-support/supported-auth-browser')
installSupportedAuthBrowser()

type Values = React.ComponentProps<typeof AntiRelaySection>['defaultValues']
type RequestRecord = {
  method: string
  path: string
  body: Record<string, unknown>
  actor: number | undefined
  sid: string | undefined
}
const originalAdapter = api.defaults.adapter
let server: Record<string, string>
let requests: RequestRecord[]
let transport: (request: RequestRecord) => Promise<unknown>
const groupKey = 'group_ratio_setting.group_warnings'
const initialAnti: Values = {
  AntiRelayEnabled: false,
  AntiRelayRejectProxyHeadersEnabled: false,
  AntiRelayHTTPSOnlyEnabled: true,
  AntiRelayBlockedCIDRs: ['198.51.100.1'],
  AntiRelayTrustedProxyCIDRs: ['127.0.0.1'],
}
function optionsResponse() {
  return {
    success: true,
    message: '',
    data: Object.entries(server).map(([key, value]) => ({ key, value })),
    capabilities: {
      read_only: false,
      writable_keys: [...Object.keys(initialAnti), groupKey, 'GroupRatio'],
    },
  }
}
beforeEach(() => {
  componentWriteBoundary = false
  actor()
  server = { [groupKey]: '{"initial":1}', GroupRatio: '{"other":2}' }
  requests = []
  transport = async (request) => {
    if (request.method === 'get' && request.path === '/api/option/') {
      return optionsResponse()
    }
    if (request.method === 'get' && request.path === '/api/site/navigation') {
      return { success: true, message: '', data: { value: '{}', revision: 1 } }
    }
    if (request.method === 'post' && request.path === '/api/option/validate') {
      return { success: true, message: '' }
    }
    if (request.method === 'post' && request.path === '/api/option/bulk') {
      Object.assign(server, request.body.values)
      return { success: true, message: '' }
    }
    if (request.method === 'put' && request.path === '/api/option/') {
      server[String(request.body.key)] = String(request.body.value)
      return { success: true, message: '' }
    }
    unexpected.push(`${request.method} ${request.path}`)
    throw new Error('Unexpected settings transport')
  }
  api.defaults.adapter = async (config) => {
    const owner = useAuthStore.getState().auth
    const request = {
      method: config.method ?? '',
      path: config.url ?? '',
      body:
        typeof config.data === 'string'
          ? JSON.parse(config.data)
          : (config.data ?? {}),
      actor: owner.user?.id,
      sid: owner.session?.sid,
    }
    requests.push(request)
    const data = await transport(request)
    return { config, data, status: 200, statusText: 'fixture', headers: {} }
  }
})
afterEach(() => {
  api.defaults.adapter = originalAdapter
})
function editor() {
  const node = document.querySelector<HTMLTextAreaElement>('[data-editor]')
  assert.ok(node)
  return node
}
function dirty() {
  return (
    document.querySelector('[data-dirty]')?.getAttribute('data-dirty') ===
    'true'
  )
}
async function refresh(
  ui: Awaited<ReturnType<typeof mount>>,
  values: Record<string, string>
) {
  server = values
  await act(async () =>
    ui.client.setQueryData(['system-options'], optionsResponse())
  )
  await flush()
}
async function selectConfiguration(key: string) {
  const trigger = document.querySelector<HTMLElement>('[role="combobox"]')
  assert.ok(trigger)
  await click(trigger)
  const option = [
    ...document.querySelectorAll<HTMLElement>('[role="option"]'),
  ].find((item) => item.textContent?.includes(key))
  assert.ok(option)
  await click(option)
}
function Observer({ children }: { children: React.ReactNode }) {
  useSystemOptions()
  return <>{children}</>
}
function anti(values: Values = initialAnti) {
  return (
    <Observer>
      <AntiRelaySection defaultValues={values} />
    </Observer>
  )
}
function blocked() {
  const node = document.querySelector<HTMLTextAreaElement>(
    'textarea[name="AntiRelayBlockedCIDRs"]'
  )
  assert.ok(node)
  return node
}
function trusted() {
  const node = document.querySelector<HTMLTextAreaElement>(
    'textarea[name="AntiRelayTrustedProxyCIDRs"]'
  )
  assert.ok(node)
  return node
}
function enabledSwitch(host: HTMLElement) {
  const node = host.querySelector<HTMLElement>('[role="switch"]')
  assert.ok(node)
  return node
}
const writes = () => requests.filter((request) => request.method !== 'get')

test('Ask AI to edit uses the interface language and preserves the JSON draft', async () => {
  const { consumeQueuedAssistantRequest } =
    await import('@/features/assistant/assistant-events')
  const french = (await import('@/i18n/locales/fr.json')).default.translation
  const previousLanguage = i18n.language
  const draft = '{"comment":"<A&B> {{literal}}"}'
  await mount(<RawJsonConfigurationSection />)
  await until(
    () => editor().value.includes('initial'),
    'initial source hydration'
  )
  await edit(editor(), draft)
  const ask = button('Ask AI to edit')
  const prompt = async () => {
    await click(ask)
    const request = consumeQueuedAssistantRequest()
    assert.equal(request?.autoSend, false)
    assert.ok(request?.message)
    assert.ok(request.message.endsWith(`\n\n${draft}`))
    return request.message
  }
  try {
    assert.ok(
      (await prompt()).startsWith(`Help me edit the ${groupKey} admin configuration.`)
    )
    i18n.addResourceBundle('fr', 'translation', french)
    await act(async () => {
      await i18n.changeLanguage('fr')
    })
    assert.ok(
      (await prompt()).startsWith(
        `Aide-moi à modifier la configuration d’administration ${groupKey}.`
      )
    )
    assert.equal(writes().length, 0)
  } finally {
    await act(async () => {
      await i18n.changeLanguage(previousLanguage)
    })
    i18n.removeResourceBundle('fr', 'translation')
    i18n.addResourceBundle('fr', 'translation', {})
  }
})

test('raw JSON applies clean server updates, retains dirty source changes/equal refreshes and confirms key replacement', async () => {
  const ui = await mount(<RawJsonConfigurationSection />)
  await until(
    () => editor().value.includes('initial'),
    'initial source hydration'
  )
  await refresh(ui, { ...server, [groupKey]: '{"server":2}' })
  assert.deepEqual(JSON.parse(editor().value), { server: 2 })
  await edit(editor(), '{"draft":3}')
  await refresh(ui, { ...server })
  assert.deepEqual(JSON.parse(editor().value), { draft: 3 })
  await refresh(ui, { ...server, [groupKey]: '{"server":4}' })
  assert.deepEqual(JSON.parse(editor().value), { draft: 3 })
  assert.equal(dirty(), true)
  await selectConfiguration('GroupRatio')
  await click(button('Cancel replacement'))
  assert.equal(editor().getAttribute('data-editor'), groupKey)
  assert.deepEqual(JSON.parse(editor().value), { draft: 3 })
  await selectConfiguration('GroupRatio')
  await click(button('Confirm replacement'))
  assert.equal(editor().getAttribute('data-editor'), 'GroupRatio')
  assert.deepEqual(JSON.parse(editor().value), { other: 2 })
  assert.equal(dirty(), false)
  assert.equal(writes().length, 0)
})

test('raw JSON falls back only when clean and does not discard a draft whose server key disappears', async () => {
  server = { GroupRatio: '{"ratio":1}' }
  const ui = await mount(<RawJsonConfigurationSection />)
  await until(
    () =>
      editor().getAttribute('data-editor') === 'GroupRatio' &&
      editor().value.includes('ratio'),
    'first available key'
  )
  await edit(editor(), '{"pending":2}')
  await refresh(ui, { [groupKey]: '{"new":3}' })
  assert.equal(editor().getAttribute('data-editor'), 'GroupRatio')
  assert.deepEqual(JSON.parse(editor().value), { pending: 2 })
  assert.equal(dirty(), true)
  await edit(editor(), JSON.stringify({ ratio: 1 }, null, 2))
  await until(
    () => editor().getAttribute('data-editor') === groupKey,
    'clean fallback'
  )
  assert.deepEqual(JSON.parse(editor().value), { new: 3 })
})

test('raw JSON keeps failed drafts and validates before saving exactly the selected JSON value', async () => {
  const base = transport
  let rejectSave = true
  transport = async (request) => {
    if (request.path === '/api/option/bulk' && rejectSave) {
      return { success: false, message: 'synthetic save rejection' }
    }
    return base(request)
  }
  const ui = await mount(<RawJsonConfigurationSection />)
  await until(() => !editor().disabled, 'editor ready')
  await edit(editor(), '{')
  await click(button('Save Changes', ui.actions))
  assert.equal(writes().length, 0)
  assert.match(ui.host.textContent ?? '', /Invalid JSON/)
  await edit(editor(), '{"saved":5}')
  await click(button('Save Changes', ui.actions))
  await until(
    () =>
      messages.some(
        (message) => message.message === 'synthetic save rejection'
      ),
    'save rejection shown'
  )
  assert.equal(dirty(), true)
  assert.equal(editor().value, '{"saved":5}')
  rejectSave = false
  await click(button('Save Changes', ui.actions))
  await until(() => !dirty() && !editor().disabled, 'save acknowledged')
  assert.deepEqual(JSON.parse(editor().value), { saved: 5 })
  assert.deepEqual(
    writes().map(({ path, body }) => ({ path, body })),
    [
      {
        path: '/api/option/validate',
        body: { values: { [groupKey]: '{"saved":5}' } },
      },
      {
        path: '/api/option/bulk',
        body: { values: { [groupKey]: '{"saved":5}' } },
      },
      {
        path: '/api/option/validate',
        body: { values: { [groupKey]: '{"saved":5}' } },
      },
      {
        path: '/api/option/bulk',
        body: { values: { [groupKey]: '{"saved":5}' } },
      },
    ]
  )
})

for (const scope of ['actor', 'session'] as const) {
  test(`raw JSON ${scope} remount clears draft and late validation cannot start a write`, async () => {
    const pending = deferred(),
      base = transport
    transport = async (request) =>
      request.path === '/api/option/validate' ? pending.promise : base(request)
    const ui = await mount(<RawJsonConfigurationSection />)
    await until(() => !editor().disabled, 'editor ready')
    await edit(editor(), '{"private":6}')
    await click(button('Save Changes', ui.actions))
    assert.equal(writes().length, 1)
    await act(async () => actor(scope === 'actor' ? 42 : 41, 'session-B'))
    assert.deepEqual(JSON.parse(editor().value), { initial: 1 })
    await act(async () => pending.resolve({ success: true }))
    await flush()
    assert.equal(writes().length, 1)
    assert.equal(
      messages.filter((message) => message.kind === 'success').length,
      0
    )
    assert.equal(dirty(), false)
  })
}

test('raw JSON delayed import consults the latest draft, permits cancel, and ignores a previous account import', async () => {
  const ui = await mount(<RawJsonConfigurationSection />)
  await until(() => !editor().disabled, 'editor ready')
  const pending = deferred<string>()
  const upload = ui.host.querySelector<HTMLInputElement>('input[type="file"]')
  assert.ok(upload)
  Object.defineProperty(upload, 'files', {
    configurable: true,
    value: [{ text: () => pending.promise }],
  })
  await act(async () =>
    upload.dispatchEvent(new Event('change', { bubbles: true }))
  )
  await edit(editor(), '{"typed-after-import":7}')
  await act(async () => pending.resolve('{"import":8}'))
  await click(button('Cancel replacement'))
  assert.deepEqual(JSON.parse(editor().value), { 'typed-after-import': 7 })
  const obsolete = deferred<string>()
  Object.defineProperty(upload, 'files', {
    configurable: true,
    value: [{ text: () => obsolete.promise }],
  })
  await act(async () =>
    upload.dispatchEvent(new Event('change', { bubbles: true }))
  )
  await act(async () => actor(42, 'session-B'))
  await act(async () => obsolete.resolve('{"old-account":9}'))
  await flush()
  assert.deepEqual(JSON.parse(editor().value), { initial: 1 })
  assert.equal(document.querySelector('[role="dialog"]'), null)
})

test('anti-relay retains equal-default and dirty fields while refreshed untouched fields follow the server', async () => {
  const ui = await mount(anti())
  await until(
    () => ui.client.getQueryState(['system-options'])?.status === 'success',
    'capabilities ready'
  )
  await edit(blocked(), '203.0.113.0/24')
  await ui.render(
    anti({
      ...initialAnti,
      AntiRelayBlockedCIDRs: [...initialAnti.AntiRelayBlockedCIDRs],
    })
  )
  assert.equal(blocked().value, '203.0.113.0/24')
  await ui.render(
    anti({
      ...initialAnti,
      AntiRelayBlockedCIDRs: ['192.0.2.1'],
      AntiRelayTrustedProxyCIDRs: ['::1'],
    })
  )
  assert.equal(blocked().value, '203.0.113.0/24')
  assert.equal(trusted().value, '::1')
  assert.equal(writes().length, 0)
})

test('anti-relay component unit validates event submission, keeps policy save order, and retains a rejected draft for explicit retry', async () => {
  componentWriteBoundary = true
  const base = transport
  let fail = true
  transport = async (request) =>
    request.method === 'put' && fail
      ? { success: false, message: 'synthetic policy rejection' }
      : base(request)
  const ui = await mount(anti())
  await until(
    () => ui.client.getQueryState(['system-options'])?.status === 'success',
    'capabilities ready'
  )
  await edit(blocked(), '203.0.113.1\n 203.0.113.2\n')
  await click(enabledSwitch(ui.host))
  await click(button('Save anti-relay settings', ui.actions))
  await until(
    () =>
      ui.host.textContent?.includes(
        'Anti-relay settings could not be saved'
      ) === true,
    'error feedback'
  )
  assert.equal(blocked().value, '203.0.113.1\n 203.0.113.2\n')
  assert.deepEqual(
    writes().map((request) => request.body.key),
    ['AntiRelayBlockedCIDRs']
  )
  fail = false
  await click(button('Save anti-relay settings', ui.actions))
  await until(
    () => ui.host.textContent?.includes('Anti-relay settings saved') === true,
    'save feedback'
  )
  assert.deepEqual(
    writes()
      .slice(1)
      .map((request) => request.body),
    [
      { key: 'AntiRelayBlockedCIDRs', value: '["203.0.113.1","203.0.113.2"]' },
      { key: 'AntiRelayEnabled', value: true },
    ]
  )
  assert.equal(blocked().value, '203.0.113.1\n203.0.113.2')
})

test('anti-relay component unit keeps edits made during a save pending for the next explicit save', async () => {
  componentWriteBoundary = true
  const pending = deferred(),
    base = transport
  let hold = true
  transport = async (request) =>
    request.method === 'put' && hold ? pending.promise : base(request)
  const ui = await mount(anti())
  await until(
    () => ui.client.getQueryState(['system-options'])?.status === 'success',
    'capabilities ready'
  )
  await edit(blocked(), '203.0.113.1')
  await click(button('Save anti-relay settings', ui.actions))
  await until(() => writes().length === 1, 'save in flight')
  await edit(blocked(), '203.0.113.2')
  await act(async () => pending.resolve({ success: true, message: '' }))
  await until(
    () => !button('Save anti-relay settings', ui.actions).disabled,
    'save settled'
  )
  assert.equal(blocked().value, '203.0.113.2')
  assert.doesNotMatch(ui.host.textContent ?? '', /Anti-relay settings saved/)
  hold = false
  await click(button('Save anti-relay settings', ui.actions))
  await until(
    () =>
      writes().length === 2 &&
      ui.host.textContent?.includes('Anti-relay settings saved') === true,
    'next save acknowledges latest edit'
  )
  assert.deepEqual(
    writes().map((request) => request.body.value),
    ['["203.0.113.1"]', '["203.0.113.2"]']
  )
})

test('anti-relay component unit session remount drops its draft and a late first save cannot dispatch another policy field', async () => {
  componentWriteBoundary = true
  const pending = deferred(),
    base = transport
  transport = async (request) =>
    request.method === 'put' ? pending.promise : base(request)
  const ui = await mount(anti())
  await until(
    () => ui.client.getQueryState(['system-options'])?.status === 'success',
    'capabilities ready'
  )
  await edit(blocked(), '203.0.113.1')
  await click(enabledSwitch(ui.host))
  await click(button('Save anti-relay settings', ui.actions))
  await until(() => writes().length === 1, 'first policy write in flight')
  await act(async () => actor(41, 'session-B'))
  assert.equal(blocked().value, '198.51.100.1')
  await act(async () => pending.resolve({ success: true, message: '' }))
  await flush()
  assert.equal(writes().length, 1)
  assert.equal(writes()[0].sid, 'session-A')
  assert.doesNotMatch(ui.host.textContent ?? '', /Anti-relay settings saved/)
})

test('anti-relay real capability guard rejects unsupported policy writes and preserves the draft', async () => {
  const ui = await mount(anti())
  await until(
    () => ui.client.getQueryState(['system-options'])?.status === 'success',
    'capabilities ready'
  )
  await edit(blocked(), '203.0.113.1')
  await click(button('Save anti-relay settings', ui.actions))
  await until(
    () =>
      ui.host.textContent?.includes(
        'Anti-relay settings could not be saved'
      ) === true,
    'real guard error'
  )
  assert.equal(blocked().value, '203.0.113.1')
  assert.equal(writes().length, 0)
  assert.ok(
    messages.some(
      (message) => message.kind === 'error' && message.message === 'Read only'
    )
  )
})

test('raw JSON server validation rejection performs no bulk write and retains its draft', async () => {
  const base = transport
  transport = async (request) =>
    request.path === '/api/option/validate'
      ? { success: false, message: 'synthetic schema rejection' }
      : base(request)
  const ui = await mount(<RawJsonConfigurationSection />)
  await until(() => !editor().disabled, 'editor ready')
  await edit(editor(), '{"invalid-schema":1}')
  await click(button('Save Changes', ui.actions))
  await until(
    () => ui.host.textContent?.includes('synthetic schema rejection') === true,
    'server rejection shown'
  )
  assert.deepEqual(
    writes().map((request) => request.path),
    ['/api/option/validate']
  )
  assert.equal(editor().value, '{"invalid-schema":1}')
  assert.equal(dirty(), true)
})

test('raw JSON acknowledged save survives a failed refresh without bouncing to the cached server value', async () => {
  const base = transport
  let acknowledged = false
  transport = async (request) => {
    if (request.path === '/api/option/bulk') {
      acknowledged = true
      return { success: true, message: '' }
    }
    if (acknowledged && request.method === 'get') {
      throw new Error('synthetic refresh outage')
    }
    return base(request)
  }
  const ui = await mount(<RawJsonConfigurationSection />)
  await until(() => !editor().disabled, 'editor ready')
  await edit(editor(), '{"accepted":1}')
  await click(button('Save Changes', ui.actions))
  await until(
    () => ui.host.textContent?.includes('synthetic refresh outage') === true,
    'refresh failure shown'
  )
  assert.equal(editor().value, '{"accepted":1}')
  assert.equal(dirty(), false)
  assert.equal(writes().length, 2)
  assert.equal(
    messages.filter((message) => message.kind === 'success').length,
    1
  )
})

test('raw JSON StrictMode effect replay permits normal validation and save', async () => {
  const ui = await mount(
    <React.StrictMode>
      <RawJsonConfigurationSection />
    </React.StrictMode>
  )
  await until(() => !editor().disabled, 'strict editor ready')
  await edit(editor(), '{"strict":1}')
  await click(button('Save Changes', ui.actions))
  await until(() => writes().length === 2 && !dirty(), 'strict save')
  assert.deepEqual(
    writes().map((request) => request.path),
    ['/api/option/validate', '/api/option/bulk']
  )
})

test('anti-relay component unit disables first and then changes lists under StrictMode', async () => {
  componentWriteBoundary = true
  const ui = await mount(
    <React.StrictMode>
      {anti({ ...initialAnti, AntiRelayEnabled: true })}
    </React.StrictMode>
  )
  await until(
    () => ui.client.getQueryState(['system-options'])?.status === 'success',
    'capabilities ready'
  )
  await click(enabledSwitch(ui.host))
  await edit(blocked(), '203.0.113.1')
  await click(button('Save anti-relay settings', ui.actions))
  await until(
    () => ui.host.textContent?.includes('Anti-relay settings saved') === true,
    'strict save acknowledged'
  )
  assert.deepEqual(
    writes().map((request) => request.body),
    [
      { key: 'AntiRelayEnabled', value: false },
      { key: 'AntiRelayBlockedCIDRs', value: '["203.0.113.1"]' },
    ]
  )
})

test('anti-relay component unit source refresh preserves edits but stops obsolete remaining save steps', async () => {
  componentWriteBoundary = true
  const pending = deferred(),
    base = transport
  transport = async (request) =>
    request.method === 'put' ? pending.promise : base(request)
  const ui = await mount(anti())
  await until(
    () => ui.client.getQueryState(['system-options'])?.status === 'success',
    'capabilities ready'
  )
  await edit(blocked(), '203.0.113.1')
  await click(enabledSwitch(ui.host))
  await click(button('Save anti-relay settings', ui.actions))
  await until(() => writes().length === 1, 'first policy write')
  await ui.render(anti({ ...initialAnti, AntiRelayTrustedProxyCIDRs: ['::1'] }))
  await act(async () => pending.resolve({ success: true, message: '' }))
  await flush()
  assert.equal(writes().length, 1)
  assert.equal(blocked().value, '203.0.113.1')
  assert.equal(trusted().value, '::1')
  assert.doesNotMatch(ui.host.textContent ?? '', /Anti-relay settings saved/)
})

test('raw JSON dirty key disappearance during validation retains its draft and current validation result', async () => {
  const pending = deferred(),
    base = transport
  transport = async (request) =>
    request.path === '/api/option/validate' ? pending.promise : base(request)
  const ui = await mount(<RawJsonConfigurationSection />)
  await until(() => !editor().disabled, 'editor ready')
  await edit(editor(), '{"pending-draft":1}')
  await click(button('Validate configuration'))
  await until(() => writes().length === 1, 'validation pending')
  await refresh(ui, { GroupRatio: '{"replacement":1}' })
  assert.equal(editor().getAttribute('data-editor'), groupKey)
  assert.equal(editor().disabled, true)
  assert.deepEqual(JSON.parse(editor().value), { 'pending-draft': 1 })
  await act(async () =>
    pending.resolve({ success: false, message: 'current validation rejection' })
  )
  await until(() => !editor().disabled, 'validation settled')
  assert.deepEqual(JSON.parse(editor().value), { 'pending-draft': 1 })
  assert.match(ui.host.textContent ?? '', /current validation rejection/)
  assert.equal(dirty(), true)
  assert.deepEqual(
    writes().map(({ path, body }) => ({ path, body })),
    [
      {
        path: '/api/option/validate',
        body: { values: { [groupKey]: '{"pending-draft":1}' } },
      },
    ]
  )
})

test('anti-relay component unit equal values in reordered props retain the acknowledged save baseline', async () => {
  componentWriteBoundary = true
  const ui = await mount(anti())
  await until(
    () => ui.client.getQueryState(['system-options'])?.status === 'success',
    'capabilities ready'
  )
  await edit(blocked(), '203.0.113.1')
  await click(button('Save anti-relay settings', ui.actions))
  await until(
    () => ui.host.textContent?.includes('Anti-relay settings saved') === true,
    'first save'
  )
  await ui.render(
    anti({
      AntiRelayTrustedProxyCIDRs: ['127.0.0.1'],
      AntiRelayBlockedCIDRs: ['198.51.100.1'],
      AntiRelayHTTPSOnlyEnabled: true,
      AntiRelayRejectProxyHeadersEnabled: false,
      AntiRelayEnabled: false,
    })
  )
  assert.equal(blocked().value, '203.0.113.1')
  await edit(trusted(), '::1')
  await click(button('Save anti-relay settings', ui.actions))
  await until(
    () => ui.host.textContent?.includes('Anti-relay settings saved') === true,
    'second save'
  )
  assert.deepEqual(
    writes().map((request) => request.body.key),
    ['AntiRelayBlockedCIDRs', 'AntiRelayTrustedProxyCIDRs']
  )
})
