/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, beforeEach, describe, test } from 'node:test'

import {
  AxiosError,
  type AxiosResponse,
  type InternalAxiosRequestConfig,
} from 'axios'
import { Window } from 'happy-dom'

import type { ReleaseNote } from './types'

const dom = new Window({
  url: 'https://console.example.test/operations/releases',
})
const originalGlobals = new Map<string, PropertyDescriptor | undefined>()

function installGlobal(name: string, value: unknown) {
  originalGlobals.set(name, Object.getOwnPropertyDescriptor(globalThis, name))
  Object.defineProperty(globalThis, name, {
    configurable: true,
    writable: true,
    value,
  })
}

for (const name of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLInputElement',
  'HTMLTextAreaElement',
  'HTMLButtonElement',
  'HTMLFormElement',
  'Element',
  'SVGElement',
  'Node',
  'Event',
  'MouseEvent',
  'PointerEvent',
  'FocusEvent',
  'CustomEvent',
  'MutationObserver',
  'ResizeObserver',
] as const) {
  installGlobal(name, name === 'window' ? dom : dom[name])
}
installGlobal('getComputedStyle', dom.getComputedStyle.bind(dom))
installGlobal('requestAnimationFrame', (callback: FrameRequestCallback) =>
  setTimeout(() => callback(0), 0)
)
installGlobal('cancelAnimationFrame', (handle: number) => clearTimeout(handle))
installGlobal('IS_REACT_ACT_ENVIRONMENT', true)

const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { toast } = await import('sonner')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { ReleaseNotesAdmin } = await import('./admin-page')

const originalAdapter = api.defaults.adapter
const originalAuth = useAuthStore.getState().auth
const originalToastError = toast.error
const originalToastSuccess = toast.success
const i18n = createInstance()
await i18n.use(initReactI18next).init({
  lng: 'en',
  resources: { en: { translation: {} } },
  interpolation: { escapeValue: false },
})

type Request = InternalAxiosRequestConfig
let requests: Request[] = []
let toastEvents: string[] = []
let dispose: (() => Promise<void>) | undefined

function login(userId = 901, sessionId = 'session-a', role = 10) {
  useAuthStore.getState().auth.setBundle({
    access_token: `lmms_${'a'.repeat(48)}_${userId}_${sessionId}`,
    token_type: 'Bearer',
    access_expires_at: 2_000_000_000,
    user: { id: userId, username: `user-${userId}`, role },
    session: {
      sid: sessionId,
      current: true,
      login_method: 'password',
      ip: '127.0.0.1',
      user_agent: 'synthetic-test-browser',
      created_at: 1,
      last_active_at: 1,
      expires_at: 2_000_000_000,
    },
  })
}

function note(overrides: Partial<ReleaseNote> = {}): ReleaseNote {
  return {
    id: 101,
    version: 'v1.0.0',
    revision: 1,
    content: '- A published change',
    published_at: 1_700_000_000,
    published_by: 901,
    ...overrides,
  }
}

function raw(config: Request, data: unknown): AxiosResponse {
  return { config, headers: {}, status: 200, statusText: 'OK', data }
}

function ok(config: Request, data: unknown) {
  return raw(config, { success: true, data })
}

function unavailable(config: Request) {
  return new AxiosError(
    'synthetic outage',
    'ERR_BAD_RESPONSE',
    config,
    undefined,
    {
      ...raw(config, {
        success: false,
        code: 'OPERATIONS_UNAVAILABLE',
        message: 'synthetic outage',
      }),
      status: 503,
      statusText: 'Unavailable',
    }
  )
}

function backend(
  handle: (config: Request) => AxiosResponse | Promise<AxiosResponse>
) {
  api.defaults.adapter = async (config) => {
    const url = new URL(api.getUri(config), dom.location.href)
    assert.equal(url.pathname, '/api/release-notes/admin')
    assert.ok(config.method === 'get' || config.method === 'post')
    if (config.method === 'get') {
      assert.equal(url.searchParams.get('limit'), '50')
    } else {
      assert.equal(url.search, '')
    }
    const auth = useAuthStore.getState().auth
    assert.equal(
      config.headers.get('Authorization'),
      `Bearer ${auth.accessToken}`
    )
    assert.equal(config.authScope?.userId, auth.user?.id)
    assert.equal(config.authScope?.sessionId, auth.session?.sid)
    requests.push(config)
    return handle(config)
  }
}

function count(method: 'get' | 'post') {
  return requests.filter((request) => request.method === method).length
}

function historyKey(userId = 901, sessionId = 'session-a', role = 10) {
  return ['release-notes', 'admin', userId, sessionId, role] as const
}

const releasePending = new Set<() => void>()
function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (error: unknown) => void
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise
    reject = rejectPromise
  })
  const release = () => reject(new Error('Unfinished synthetic request'))
  releasePending.add(release)
  return {
    promise,
    resolve: (value: T) => {
      releasePending.delete(release)
      resolve(value)
    },
    reject: (error: unknown) => {
      releasePending.delete(release)
      reject(error)
    },
  }
}

async function flush() {
  await new Promise((resolve) => setTimeout(resolve, 10))
}

async function until(check: () => boolean) {
  for (let attempt = 0; attempt < 200; attempt++) {
    if (check()) return
    await act(flush)
  }
  assert.ok(
    check(),
    `Release management did not settle: ${document.body.textContent?.slice(0, 1200)}`
  )
}

async function mount() {
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: Infinity },
      // The component must explicitly disable automatic publication retries.
      mutations: { retry: 2, retryDelay: 0, gcTime: Infinity },
    },
  })
  dispose = async () => {
    await act(async () => root.unmount())
    client.clear()
    container.remove()
  }
  await act(async () => {
    root.render(
      <QueryClientProvider client={client}>
        <I18nextProvider i18n={i18n}>
          <ReleaseNotesAdmin />
        </I18nextProvider>
      </QueryClientProvider>
    )
  })
  return { container, client }
}

function field(container: HTMLElement, text: 'Version' | 'Changelog') {
  const label = [...container.querySelectorAll('label')].find(
    (element) => element.textContent?.trim() === text
  )
  assert.ok(label, `Missing accessible label: ${text}`)
  const input = document.getElementById(label.htmlFor)
  assert.ok(
    input instanceof HTMLInputElement || input instanceof HTMLTextAreaElement
  )
  return input
}

async function setField(
  container: HTMLElement,
  name: 'Version' | 'Changelog',
  value: string
) {
  const input = field(container, name)
  const prototype =
    input instanceof HTMLInputElement
      ? HTMLInputElement.prototype
      : HTMLTextAreaElement.prototype
  const setter = Object.getOwnPropertyDescriptor(prototype, 'value')?.set
  assert.ok(setter)
  await act(async () => {
    setter.call(input, value)
    input.dispatchEvent(new Event('input', { bubbles: true }))
    input.dispatchEvent(new Event('change', { bubbles: true }))
  })
}

async function fill(
  container: HTMLElement,
  draft: { version: string; content: string }
) {
  await setField(container, 'Version', draft.version)
  await setField(container, 'Changelog', draft.content)
}

async function click(container: HTMLElement, text: string) {
  const button = [...container.querySelectorAll('button')].find(
    (element) => element.textContent?.trim() === text
  )
  assert.ok(button, `Missing button: ${text}`)
  assert.equal(button.disabled, false, `Disabled button: ${text}`)
  await act(async () => button.click())
}

beforeEach(() => {
  requests = []
  toastEvents = []
  toast.error = (() => {
    toastEvents.push('error')
    return 'synthetic-toast'
  }) as typeof toast.error
  toast.success = (() => {
    toastEvents.push('success')
    return 'synthetic-toast'
  }) as typeof toast.success
})

afterEach(async () => {
  await dispose?.()
  dispose = undefined
  useAuthStore.setState({ auth: originalAuth })
  for (const release of releasePending) release()
  releasePending.clear()
  await flush()
  api.defaults.adapter = originalAdapter
  toast.error = originalToastError
  toast.success = originalToastSuccess
  dom.localStorage.clear()
  document.body.replaceChildren()
})

after(() => {
  dom.close()
  for (const [name, descriptor] of originalGlobals) {
    if (descriptor) {
      Object.defineProperty(globalThis, name, descriptor)
    } else {
      Reflect.deleteProperty(globalThis, name)
    }
  }
})

describe('release note management', { concurrency: false }, () => {
  test('previews the draft, publishes exact content, and refreshes history without invalidating unread notes', async () => {
    login()
    const draft = {
      version: 'v2.1.0',
      content: '  **A real preview**\n\n- Ready.\n',
    }
    const published = note({ ...draft, id: 102 })
    let saved = false
    backend((config) => {
      if (config.method === 'get') return ok(config, saved ? [published] : [])
      assert.deepEqual(JSON.parse(config.data), draft)
      saved = true
      return ok(config, published)
    })
    const { container, client } = await mount()
    await until(
      () => container.textContent?.includes('No version updates') === true
    )
    const unreadKey = ['release-notes', 'latest-unread', 901, 'session-a', 10]
    client.setQueryData(unreadKey, null)
    await fill(container, draft)
    const preview = [...container.querySelectorAll('h3')]
      .find((element) => element.textContent === 'Preview')
      ?.closest('section')
    assert.equal(
      preview?.querySelector('strong')?.textContent,
      'A real preview'
    )
    await click(container, 'Publish update')
    await until(
      () =>
        count('get') === 2 &&
        container.textContent?.includes(
          'Version v2.1.0 revision 1 published'
        ) === true
    )
    await until(() => client.isFetching() === 0)
    assert.equal(count('post'), 1)
    assert.deepEqual(client.getQueryData(historyKey()), [published])
    assert.match(
      container.querySelector('details summary')?.textContent ?? '',
      /v2.1.0/
    )
    assert.equal(client.getQueryState(unreadKey)?.isInvalidated, false)
  })

  test('uses the replayed server receipt without inventing another revision or publisher', async () => {
    login()
    const original = note({ revision: 7, published_by: 77 })
    backend((config) =>
      ok(config, config.method === 'get' ? [original] : original)
    )
    const { container, client } = await mount()
    await until(() => container.querySelector('details') !== null)
    await fill(container, original)
    await click(container, 'Publish update')
    await until(
      () =>
        container.textContent?.includes(
          'Version v1.0.0 revision 7 published'
        ) === true && count('get') === 2
    )
    await until(() => client.isFetching() === 0)
    assert.equal(count('post'), 1)
    assert.deepEqual(client.getQueryData(historyKey()), [original])
    assert.doesNotMatch(container.textContent ?? '', /revision 8 published/)
  })

  test('cancels an initial history read that is still in flight before refreshing after publication', async () => {
    login()
    const pending = deferred<AxiosResponse>()
    let initialRequest: Request | undefined
    const published = note({ id: 103, version: 'v-new' })
    backend((config) => {
      if (config.method === 'post') return ok(config, published)
      if (!initialRequest) {
        initialRequest = config
        return pending.promise
      }
      return ok(config, [published])
    })
    const { container, client } = await mount()
    await until(() => initialRequest !== undefined)
    await fill(container, published)
    await click(container, 'Publish update')
    await until(() => count('get') === 2 && client.isFetching() === 0)
    assert.equal(initialRequest?.signal?.aborted, true)
    assert.deepEqual(client.getQueryData(historyKey()), [published])
    await act(async () => {
      pending.resolve(ok(initialRequest!, [note({ version: 'v-obsolete' })]))
      await flush()
    })
    assert.deepEqual(client.getQueryData(historyKey()), [published])
    assert.doesNotMatch(container.textContent ?? '', /v-obsolete/)
    assert.match(
      container.textContent ?? '',
      /Version v-new revision 1 published/
    )
  })

  test('preserves a failed draft and retries publication only after another explicit click', async () => {
    login()
    const draft = { version: 'v-retry', content: '- Keep this draft' }
    const published = note(draft)
    let fail = true
    let saved = false
    backend((config) => {
      if (config.method === 'get') return ok(config, saved ? [published] : [])
      if (fail) throw new AxiosError('lost reply', 'ERR_NETWORK', config)
      saved = true
      return ok(config, published)
    })
    const { container, client } = await mount()
    await until(() => client.isFetching() === 0)
    await fill(container, draft)
    await click(container, 'Publish update')
    await until(
      () =>
        container.textContent?.includes('Request failed') === true &&
        client.isMutating() === 0
    )
    assert.equal(count('post'), 1)
    assert.equal(field(container, 'Version').value, draft.version)
    assert.equal(field(container, 'Changelog').value, draft.content)
    await click(container, 'Refresh')
    await until(() => count('get') === 2 && client.isFetching() === 0)
    assert.equal(count('post'), 1)
    assert.equal(field(container, 'Changelog').value, draft.content)
    fail = false
    await click(container, 'Publish update')
    await until(
      () =>
        container.textContent?.includes(
          'Version v-retry revision 1 published'
        ) === true
    )
    assert.equal(count('post'), 2)
  })

  test('rejects version, blank, NUL, and UTF-8 overflow input locally while accepting exact boundaries', async () => {
    login()
    let published: ReleaseNote | undefined
    backend((config) => {
      if (config.method === 'get')
        return ok(config, published ? [published] : [])
      published = note(JSON.parse(config.data))
      return ok(config, published)
    })
    const { container } = await mount()
    const invalid: Array<{
      version: string
      content: string
      field: 'Version' | 'Changelog'
    }> = [
      { version: '', content: 'valid', field: 'Version' },
      { version: 'v 1', content: 'valid', field: 'Version' },
      { version: 'v'.repeat(65), content: 'valid', field: 'Version' },
      { version: 'v1', content: ' \n ', field: 'Changelog' },
      { version: 'v1', content: 'before\u0000after', field: 'Changelog' },
      { version: 'v1', content: `${'é'.repeat(16_384)}a`, field: 'Changelog' },
    ]
    for (const input of invalid) {
      await fill(container, input)
      await click(container, 'Publish update')
      assert.equal(
        field(container, input.field).getAttribute('aria-invalid'),
        'true'
      )
      assert.equal(count('post'), 0)
    }
    const boundary = {
      version: `v${'a'.repeat(63)}`,
      content: 'é'.repeat(16_384),
    }
    await fill(container, boundary)
    await click(container, 'Publish update')
    await until(
      () => container.textContent?.includes('revision 1 published') === true
    )
    assert.equal(count('post'), 1)
    assert.equal(published?.version, boundary.version)
    assert.ok(published?.content === boundary.content)
    assert.equal(
      new TextEncoder().encode(published?.content).byteLength,
      32_768
    )
  })

  test('shows failed and malformed history as errors and reaches an empty list only after a valid retry', async () => {
    login()
    let reads = 0
    backend((config) => {
      assert.equal(config.method, 'get')
      reads++
      if (reads === 1) throw unavailable(config)
      if (reads === 2) return raw(config, { success: true })
      return ok(config, [])
    })
    const { container } = await mount()
    await until(
      () =>
        container.textContent?.includes(
          'extension service is unavailable (503)'
        ) === true
    )
    assert.doesNotMatch(container.textContent ?? '', /No version updates/)
    await click(container, 'Retry')
    await until(
      () =>
        container.textContent?.includes('Failed to load release history') ===
        true
    )
    assert.doesNotMatch(container.textContent ?? '', /No version updates/)
    await click(container, 'Retry')
    await until(
      () =>
        container.textContent?.includes(
          'No version updates have been published yet.'
        ) === true
    )
    assert.equal(reads, 3)
    assert.equal(count('post'), 0)
  })

  test('cancels former-account history and clears the former draft when the account changes', async () => {
    login()
    const pending = deferred<AxiosResponse>()
    let oldRequest: Request | undefined
    const current = note({
      id: 201,
      version: 'v-current-account',
      published_by: 902,
    })
    backend((config) => {
      if (config.authScope?.userId === 901) {
        oldRequest = config
        return pending.promise
      }
      return ok(config, [current])
    })
    const { container, client } = await mount()
    await until(() => oldRequest !== undefined)
    await fill(container, { version: 'v-old-draft', content: '- Former draft' })
    await act(async () => login(902, 'session-b'))
    await until(
      () => container.textContent?.includes('v-current-account') === true
    )
    assert.equal(field(container, 'Version').value, '')
    assert.equal(field(container, 'Changelog').value, '')
    assert.equal(oldRequest?.signal?.aborted, true)
    await act(async () => {
      pending.resolve(ok(oldRequest!, [note({ version: 'v-former-result' })]))
      await flush()
    })
    assert.equal(count('get'), 2)
    assert.deepEqual(client.getQueryData(historyKey(902, 'session-b')), [
      current,
    ])
    assert.doesNotMatch(
      container.textContent ?? '',
      /v-former-result|Former draft/
    )
    assert.equal(useAuthStore.getState().auth.user?.id, 902)
    assert.deepEqual(toastEvents, [])
  })

  test('a late publication success cannot refresh or replace a new session workspace', async () => {
    login()
    const pending = deferred<AxiosResponse>()
    let postRequest: Request | undefined
    const former = note({
      version: 'v-former-submit',
      content: '- Former content',
    })
    const current = note({ id: 202, version: 'v-current-session' })
    backend((config) => {
      if (config.method === 'post') {
        postRequest = config
        return pending.promise
      }
      return ok(
        config,
        config.authScope?.sessionId === 'session-a' ? [] : [current]
      )
    })
    const { container, client } = await mount()
    await fill(container, former)
    await click(container, 'Publish update')
    await until(() => postRequest !== undefined)
    await act(async () => login(901, 'session-b'))
    await until(
      () => container.textContent?.includes('v-current-session') === true
    )
    assert.equal(field(container, 'Version').value, '')
    const nextDraft = {
      version: 'v-next-draft',
      content: '- Current session draft',
    }
    await fill(container, nextDraft)
    await act(async () => {
      pending.resolve(ok(postRequest!, former))
      await flush()
    })
    await until(() => client.isMutating() === 0)
    assert.equal(count('get'), 2)
    assert.equal(count('post'), 1)
    assert.deepEqual(client.getQueryData(historyKey(901, 'session-b')), [
      current,
    ])
    assert.equal(field(container, 'Version').value, nextDraft.version)
    assert.equal(field(container, 'Changelog').value, nextDraft.content)
    assert.doesNotMatch(
      container.textContent ?? '',
      /v-former-submit|Request failed/
    )
    assert.equal(useAuthStore.getState().auth.session?.sid, 'session-b')
    assert.deepEqual(toastEvents, [])
  })

  test('a role downgrade removes management immediately and ignores a late publication failure', async () => {
    login()
    const pending = deferred<AxiosResponse>()
    let postRequest: Request | undefined
    backend((config) => {
      if (config.method === 'get') return ok(config, [])
      postRequest = config
      return pending.promise
    })
    const { container, client } = await mount()
    await fill(container, note())
    await click(container, 'Publish update')
    await until(() => postRequest !== undefined)
    await act(async () => login(901, 'session-a', 1))
    const downgradedAuth = useAuthStore.getState().auth
    assert.match(
      container.textContent ?? '',
      /current administrator session is required/
    )
    assert.equal(container.querySelector('form'), null)
    await act(async () => {
      pending.reject(unavailable(postRequest!))
      await flush()
    })
    await until(() => client.isMutating() === 0)
    assert.equal(count('get'), 1)
    assert.equal(count('post'), 1)
    assert.equal(useAuthStore.getState().auth, downgradedAuth)
    assert.doesNotMatch(
      container.textContent ?? '',
      /Request failed|synthetic outage|revision 1 published/
    )
    assert.equal(dom.location.pathname, '/operations/releases')
    assert.deepEqual(toastEvents, [])
  })

  test('denies anonymous and non-administrator roles without a request and permits exact super-admin role', async () => {
    useAuthStore.getState().auth.reset()
    backend((config) => ok(config, []))
    const { container } = await mount()
    assert.match(
      container.textContent ?? '',
      /current administrator session is required/
    )
    assert.equal(requests.length, 0)
    for (const role of [0, 1, 5, 6, 11, 99, 101]) {
      await act(async () => login(901, 'session-a', role))
      assert.match(
        container.textContent ?? '',
        /current administrator session is required/
      )
      assert.equal(container.querySelector('form'), null)
      assert.equal(requests.length, 0)
    }
    await act(async () => login(901, 'session-a', 100))
    await until(
      () =>
        container.textContent?.includes(
          'No version updates have been published yet.'
        ) === true
    )
    assert.ok(container.querySelector('form'))
    assert.equal(count('get'), 1)
    assert.equal(count('post'), 0)
  })
})
