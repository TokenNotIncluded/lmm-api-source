/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { Window } from 'happy-dom'

const dom = new Window()
// happy-dom omits the handled flag for the finished promise on cancellation:
// https://www.w3.org/TR/web-animations-1/#canceling-an-animation-section
// Keep this correction on this Window's prototype; callers still see rejection.
const cancelAnimation = dom.Animation.prototype.cancel
dom.Animation.prototype.cancel = function () {
  void this.finished.catch((error: unknown) => {
    if (!(error instanceof dom.DOMException) || error.name !== 'AbortError') {
      throw error
    }
  })
  cancelAnimation.call(this)
}
for (const name of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'Element',
  'Node',
  'Event',
  'MouseEvent',
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
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const previousAuth = useAuthStore.getState().auth
const originalRequest = api.request
const { ReadOnlyChannels } = await import('./read-only-channels')
const originalGet = api.get
const originalPut = api.put
const originalDelete = api.delete
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
let dispose: (() => Promise<void>) | undefined
afterEach(async () => {
  await dispose?.()
  dispose = undefined
  api.request = originalRequest
  useAuthStore.setState({ auth: previousAuth })
  api.get = originalGet
  api.put = originalPut
  api.delete = originalDelete
  document.body.replaceChildren()
})
after(() => dom.close())

async function mount(controlsEnabled = false) {
  if (controlsEnabled) {
    useAuthStore.getState().auth.setBundle({
      access_token: 'synthetic-channel-session-token',
      token_type: 'Bearer',
      access_expires_at: 2_000_000_000,
      user: { id: 501, role: 10, username: 'synthetic-admin' },
      session: {
        sid: 'synthetic-channel-controls',
        current: true,
        login_method: 'password',
        ip: '127.0.0.1',
        user_agent: 'synthetic',
        created_at: 1,
        last_active_at: 1,
        expires_at: 2_000_000_000,
      },
    })
  }
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  await act(async () =>
    root.render(
      <QueryClientProvider client={client}>
        <I18nextProvider i18n={i18n}>
          <ReadOnlyChannels controlsEnabled={controlsEnabled} />
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
  assert.ok(check(), 'The channel view did not reach the expected state')
}

test('animation cancellation keeps its rejected receipt and stays local to this Window', async () => {
  const otherWindow = new Window()
  try {
    assert.equal(otherWindow.Animation.prototype.cancel, cancelAnimation)
    const element = dom.document.createElement('div')
    const animation = element.animate({ opacity: [0, 1] }, { duration: 1000 })
    const finished = animation.finished
    animation.cancel()
    assert.equal(animation.playState, 'idle')
    await assert.rejects(finished, { name: 'AbortError' })
  } finally {
    otherWindow.close()
  }
})

test('read-only channels show snapshot fields and paginate without mutation controls or fake metrics', async () => {
  api.get = (async (_url: string, config?: Parameters<typeof api.get>[1]) => {
    const page = Number(config?.params?.p)
    return {
      data: {
        success: true,
        data: {
          items: [
            {
              id: page,
              name: `Channel ${page}`,
              status: page === 1 ? 1 : 2,
              group: 'vip',
              models: 'native-model',
            },
          ],
          total: 21,
        },
      },
    }
  }) as typeof api.get
  const container = await mount()
  await until(() =>
    Boolean(
      container.querySelector('table')?.textContent?.includes('Channel 1')
    )
  )
  assert.match(container.textContent ?? '', /Read only/)
  assert.match(container.textContent ?? '', /Total: 21/)
  assert.match(container.querySelector('tbody')?.textContent ?? '', /Enabled/)
  assert.deepEqual(
    [...container.querySelectorAll('button')].map(
      (button) => button.textContent
    ),
    ['Search', 'Previous', 'Next']
  )
  assert.doesNotMatch(
    container.querySelector('table')?.textContent ?? '',
    /Balance|Response Time|Actions/
  )
  const next = [...container.querySelectorAll('button')].find(
    (button) => button.textContent === 'Next'
  )
  assert.ok(next)
  await act(async () => next.click())
  await until(() =>
    Boolean(
      container.querySelector('table')?.textContent?.includes('Channel 2')
    )
  )
  assert.match(container.querySelector('tbody')?.textContent ?? '', /Disabled/)
})

test('read-only channel search submits a trimmed keyword to the read endpoint', async () => {
  const requests: { url: string; keyword?: string; page: number }[] = []
  api.get = (async (url: string, config?: Parameters<typeof api.get>[1]) => {
    requests.push({
      url,
      keyword: config?.params?.keyword,
      page: config?.params?.p,
    })
    return {
      data: {
        success: true,
        data: {
          items: [
            {
              id: 1,
              name: 'Snapshot channel',
              status: 1,
              group: 'vip',
              models: 'model',
            },
          ],
          total: 1,
        },
      },
    }
  }) as typeof api.get
  const container = await mount()
  await until(() => Boolean(container.querySelector('table')))
  const input = container.querySelector('input')
  const form = container.querySelector('form')
  assert.ok(input)
  assert.ok(form)
  const setter = Object.getOwnPropertyDescriptor(
    dom.HTMLInputElement.prototype,
    'value'
  )?.set
  assert.ok(setter)
  await act(async () => {
    setter.call(input, '  model  ')
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
  await act(async () => {
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
  })
  await until(() =>
    requests.some((request) => request.url === '/api/channel/search')
  )
  assert.deepEqual(requests.at(-1), {
    url: '/api/channel/search',
    keyword: 'model',
    page: 1,
  })
})

test('read-only channel failures keep the server reason instead of an empty successful list', async () => {
  api.get = (async () => {
    throw Object.assign(new Error('Request failed with status code 503'), {
      response: {
        data: { error: { message: 'Active routing snapshot is unavailable' } },
      },
    })
  }) as typeof api.get
  const container = await mount()
  await until(() =>
    Boolean(
      container.textContent?.includes('Active routing snapshot is unavailable')
    )
  )
  assert.equal(container.querySelector('table'), null)
  assert.doesNotMatch(container.textContent ?? '', /Total: 0|No data available/)
  assert.ok(
    [...container.querySelectorAll('button')].find(
      (button) => button.textContent === 'Retry'
    )
  )
})

test('native channel controls send the observed release and require delete confirmation', async () => {
  let version = 4
  let enabled = true
  let present = true
  const writes: { method: string; url: string; body: unknown }[] = []
  api.get = (async () => ({
    data: {
      success: true,
      data: {
        items: present
          ? [
              {
                id: 1,
                name: 'Channel 1',
                status: enabled ? 1 : 2,
                group: 'default',
                models: 'model',
              },
            ]
          : [],
        total: present ? 1 : 0,
        config_version: version,
        pricing_version: 2,
      },
    },
  })) as typeof api.get
  api.put = (async (url: string, body: unknown) => {
    writes.push({ method: 'PUT', url, body })
    enabled = false
    version += 1
    return {
      data: {
        success: true,
        data: { version: { routing: version, pricing: 2 } },
      },
    }
  }) as typeof api.put
  api.delete = (async (
    url: string,
    config?: Parameters<typeof api.delete>[1]
  ) => {
    writes.push({ method: 'DELETE', url, body: config?.data })
    present = false
    version += 1
    return {
      data: {
        success: true,
        data: { version: { routing: version, pricing: 2 } },
      },
    }
  }) as typeof api.delete
  api.request = (async (options: {
    method: string
    url: string
    data: unknown
  }) =>
    options.method === 'PUT'
      ? api.put(options.url, options.data)
      : api.delete(options.url, { data: options.data })) as typeof api.request
  const container = await mount(true)
  await until(() => Boolean(container.querySelector('table')))
  const button = (label: string) =>
    [...container.querySelectorAll<HTMLButtonElement>('table button')].find(
      (item) => item.textContent === label
    )
  assert.ok(button('Disable'))
  assert.doesNotMatch(container.textContent ?? '', /Read only/)
  await act(async () => button('Disable')!.click())
  await until(() => Boolean(button('Enable') && !button('Enable')!.disabled))
  assert.deepEqual(writes[0], {
    method: 'PUT',
    url: '/api/channel/1/status',
    body: { enabled: false, expected_version: { routing: 4, pricing: 2 } },
  })
  await act(async () => button('Delete')!.click())
  assert.equal(writes.length, 1)
  await act(async () => button('Confirm delete')!.click())
  await until(() => !container.querySelector('table tbody tr'))
  assert.deepEqual(writes[1], {
    method: 'DELETE',
    url: '/api/channel/1',
    body: { expected_version: { routing: 5, pricing: 2 } },
  })
})
