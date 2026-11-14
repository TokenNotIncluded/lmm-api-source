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
  'HTMLFormElement',
  'history',
  'location',
  'scrollTo',
  'File',
  'FileReader',
  'FormData',
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

const { useAssistantSupport } = await import('./use-assistant-support')
const { AssistantPanel } = await import('./assistant-panel')
const {
  RouterProvider,
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
} = await import('@tanstack/react-router')
const { installSupportedAuthBrowser } =
  await import('@/test-support/supported-auth-browser')
installSupportedAuthBrowser()
const originalAdapter = api.defaults.adapter
const originalFetch = globalThis.fetch
type Snapshot = ReturnType<typeof useAssistantSupport>
let latest: Snapshot
type RequestRecord = {
  method: string
  path: string
  body: Record<string, unknown>
  conversation: number
  actor: number | undefined
  sid: string | undefined
}
let requests: RequestRecord[]
let transport: (request: RequestRecord) => Promise<unknown>
function supportRequest(conversation = 91, owner = 41) {
  return {
    id: conversation,
    user_id: owner,
    conversation_id: conversation,
    kind: 'handoff' as const,
    status: 'accepted' as const,
    topic: `private topic ${owner}`,
    preferred_time: '',
    scheduled_at: 0,
    assigned_admin_id: 7,
    assigned_admin_name: 'staff',
    created_at: 1,
    updated_at: 1,
    accepted_at: 1,
    closed_at: 0,
  }
}
function Harness({
  conversation = 91,
  revision = 0,
}: {
  conversation?: number
  revision?: number
}) {
  const support = useAssistantSupport(conversation, true, revision)
  React.useEffect(() => {
    latest = support
  })
  return (
    <div data-busy={String(support.busy)}>
      <p>{support.request?.topic}</p>
      {support.messages?.map((message) => (
        <p key={message.id}>{message.content}</p>
      ))}
    </div>
  )
}
beforeEach(() => {
  actor()
  requests = []
  transport = async (request) => {
    if (request.method === 'get') {
      if (request.path === '/api/assistant/support/eligibility') {
        return { success: true, data: { eligible: true } }
      }
      if (request.path === '/api/assistant/support/self') {
        return {
          success: true,
          data: {
            request: supportRequest(request.conversation, request.actor),
          },
        }
      }
      const detail = request.path.match(/^\/api\/assistant\/support\/(\d+)$/)
      if (detail) {
        return {
          success: true,
          data: {
            request: supportRequest(Number(detail[1]), request.actor),
            messages: [
              {
                id: 1,
                role: 'admin',
                content: `private message ${request.actor}`,
                created_at: 1,
              },
            ],
          },
        }
      }
      if (request.path === '/api/assistant/status') {
        return {
          success: true,
          data: {
            enabled: true,
            model: 'synthetic-model',
            developer_access_granted: true,
            funding: { mode: 'super_administrator' },
            agent: { stream_enabled: true },
          },
        }
      }
      if (request.path === '/api/assistant/pre-conversation-presets') {
        return { success: true, data: { presets: [] } }
      }
    }
    if (request.method === 'post' && request.path.endsWith('/messages')) {
      return {
        success: true,
        data: {
          message: {
            id: 2,
            role: 'user',
            content: request.body.content,
            created_at: 1,
          },
        },
      }
    }
    unexpected.push(`${request.method} ${request.path}`)
    throw new Error('Unexpected assistant transport')
  }
  api.defaults.adapter = async (config) => {
    const auth = useAuthStore.getState().auth
    const request = {
      method: config.method ?? '',
      path: config.url ?? '',
      body:
        typeof config.data === 'string'
          ? JSON.parse(config.data)
          : (config.data ?? {}),
      conversation: Number(config.params?.conversation_id ?? 91),
      actor: auth.user?.id,
      sid: auth.session?.sid,
    }
    requests.push(request)
    return {
      config,
      data: await transport(request),
      status: 200,
      statusText: 'fixture',
      headers: {},
    }
  }
})
afterEach(() => {
  api.defaults.adapter = originalAdapter
  globalThis.fetch = originalFetch
})
const writes = () => requests.filter(({ method }) => method !== 'get')
async function ready(id = 91) {
  await until(
    () => latest?.request?.id === id && latest.messages?.length === 1,
    'support loaded'
  )
}

test('support lost-reply retry reuses its turn ID only until success in the same committed scope', async () => {
  const base = transport
  let fail = true
  transport = async (request) => {
    if (request.method === 'post' && fail) {
      fail = false
      throw new Error('synthetic lost reply')
    }
    return base(request)
  }
  await mount(<Harness />)
  await ready()
  await act(async () =>
    assert.rejects(latest.send('same content'), /synthetic lost reply/)
  )
  assert.equal(latest.busy, false)
  await act(async () => {
    await latest.send('same content')
  })
  await act(async () => {
    await latest.send('same content')
  })
  assert.equal(writes().length, 3)
  assert.equal(writes()[0].body.client_turn_id, writes()[1].body.client_turn_id)
  assert.notEqual(
    writes()[1].body.client_turn_id,
    writes()[2].body.client_turn_id
  )
  assert.ok(
    writes().every(
      ({ body, path }) =>
        body.content === 'same content' &&
        path === '/api/assistant/support/91/messages'
    )
  )
})

test('support conversation switch releases the new mutex and an old completion cannot release the new pending operation', async () => {
  const first = deferred(),
    second = deferred(),
    base = transport
  transport = async (request) =>
    request.method === 'post'
      ? request.path.includes('/91/')
        ? first.promise
        : second.promise
      : base(request)
  const ui = await mount(<Harness />)
  await ready()
  let old: ReturnType<Snapshot['send']>, current: ReturnType<Snapshot['send']>
  await act(async () => {
    old = latest.send('old private')
    void latest.send('duplicate')
  })
  assert.equal(writes().length, 1)
  assert.equal(latest.busy, true)
  await ui.render(<Harness conversation={92} />)
  await ready(92)
  assert.equal(latest.busy, false)
  await act(async () => {
    current = latest.send('new private')
  })
  assert.equal(writes().length, 2)
  await act(async () => {
    first.resolve({ success: true, data: { message: { id: 8 } } })
    assert.equal(await old, undefined)
  })
  assert.equal(latest.busy, true)
  await act(async () => {
    second.resolve({ success: true, data: { message: { id: 9 } } })
    await current
  })
  await flush()
  assert.equal(latest.busy, false)
  assert.deepEqual(
    writes().map(({ path }) => path),
    ['/api/assistant/support/91/messages', '/api/assistant/support/92/messages']
  )
})

test('support A to B to A view revisions cannot resurrect the old busy state or replay token', async () => {
  const first = deferred(),
    base = transport
  let hold = true
  transport = async (request) =>
    request.method === 'post' && hold ? first.promise : base(request)
  const ui = await mount(<Harness />)
  await ready()
  let old: ReturnType<Snapshot['send']>
  await act(async () => {
    old = latest.send('same content')
  })
  await ui.render(<Harness revision={1} />)
  await ready()
  await ui.render(<Harness revision={0} />)
  await ready()
  assert.equal(latest.busy, false)
  hold = false
  await act(async () => {
    await latest.send('same content')
  })
  assert.notEqual(
    writes()[0].body.client_turn_id,
    writes()[1].body.client_turn_id
  )
  await act(async () => {
    first.resolve({ success: true, data: { message: { id: 8 } } })
    assert.equal(await old, undefined)
  })
  assert.equal(latest.busy, false)
})

for (const boundary of ['account', 'session'] as const) {
  test(`support ${boundary} switch removes private messages and ignores a late mutation`, async () => {
    const pending = deferred(),
      base = transport
    transport = async (request) =>
      request.method === 'post' ? pending.promise : base(request)
    const ui = await mount(<Harness />)
    await ready()
    let old: ReturnType<Snapshot['send']>
    await act(async () => {
      old = latest.send('old private draft')
    })
    await act(async () => actor(boundary === 'account' ? 42 : 41, 'session-B'))
    await ready()
    assert.equal(latest.busy, false)
    assert.equal(writes().length, 1)
    assert.equal(writes()[0].actor, 41)
    assert.equal(writes()[0].sid, 'session-A')
    await act(async () => {
      pending.resolve({
        success: true,
        data: { message: { id: 8, content: 'old private draft' } },
      })
      assert.equal(await old, undefined)
    })
    assert.doesNotMatch(ui.host.textContent ?? '', /old private draft/)
    if (boundary === 'account') {
      assert.doesNotMatch(
        ui.host.textContent ?? '',
        /private message 41|private topic 41/
      )
    }
    assert.equal(latest.busy, false)
  })
}

test('panel keeps streamed markdown raw until completion, then formats it and preserves the cloud DOM lifecycle', async () => {
  const base = transport
  transport = async (request) => {
    if (request.path === '/api/assistant/support/self') {
      return { success: true, data: { request: null } }
    }
    if (
      [
        '/api/user/self/onboarding/todo',
        '/api/assistant/journey',
        '/api/assistant/new-user-gift',
        '/api/assistant/weekly-discount',
      ].includes(request.path)
    ) {
      return { success: false, message: 'not enabled in focused context' }
    }
    return base(request)
  }
  let output: ReadableStreamDefaultController<Uint8Array> | undefined
  let turn = '',
    chats = 0
  globalThis.fetch = async (input, init) => {
    assert.equal(String(input), '/api/assistant/chat')
    chats++
    const body = JSON.parse(String(init?.body))
    turn = body.client_turn_id
    return new Response(
      new ReadableStream<Uint8Array>({
        start(controller) {
          output = controller
          controller.enqueue(
            new TextEncoder().encode(
              `event: ready\ndata: ${JSON.stringify({ conversation_id: 91, client_turn_id: turn })}\n\nevent: replace\ndata: {"content":"**private streamed answer**"}\n\n`
            )
          )
        },
      }),
      { headers: { 'content-type': 'text/event-stream' } }
    )
  }
  const route = createRootRoute({
    component: () => (
      <AssistantPanel open mode='page' onOpenChange={() => {}} />
    ),
  })
  const index = createRoute({
    getParentRoute: () => route,
    path: '/',
    component: () => null,
  })
  const router = createRouter({
    routeTree: route.addChildren([index]),
    history: createMemoryHistory({ initialEntries: ['/'] }),
  })
  const ui = await mount(<RouterProvider router={router} />)
  await until(
    () =>
      document.querySelector<HTMLTextAreaElement>(
        'textarea[aria-label="Ask AI assistant"]'
      )?.disabled === false,
    'composer ready'
  )
  const cloud = document.querySelector('[data-testid="assistant-token-cloud"]')
  assert.ok(cloud)
  const composer = document.querySelector<HTMLTextAreaElement>(
    'textarea[aria-label="Ask AI assistant"]'
  )
  assert.ok(composer)
  await edit(composer, 'Explain markdown')
  const send = document.querySelector<HTMLElement>('button[aria-label="Send"]')
  assert.ok(send)
  await click(send)
  await until(
    () => ui.host.textContent?.includes('private streamed answer') === true,
    'streamed content'
  )
  assert.equal(ui.host.querySelector('strong'), null)
  assert.ok(ui.host.querySelector('.assistant-flow-response'))
  assert.equal(
    document.querySelector('[data-testid="assistant-token-cloud"]'),
    cloud
  )
  assert.equal(cloud.getAttribute('data-active'), 'true')
  assert.ok(output)
  await act(async () => {
    output?.enqueue(
      new TextEncoder().encode(
        `event: done\ndata: ${JSON.stringify({ choices: [{ message: { content: '**private streamed answer**' } }], lmm_assistant_history: { conversation_id: 91, client_turn_id: turn } })}\n\n`
      )
    )
    output?.close()
  })
  const deadline = performance.now() + 3000
  while (!ui.host.querySelector('strong') && performance.now() < deadline) {
    await flush()
  }
  assert.equal(
    ui.host.querySelector('strong')?.textContent,
    'private streamed answer'
  )
  assert.equal(composer.disabled, false)
  assert.equal(chats, 1)
  assert.equal(cloud.getAttribute('data-active'), null)
})
