/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, test } from 'node:test'

import { Window } from 'happy-dom'

const dom = new Window({
  url: 'https://console.example.test/settings/scripts',
})
dom.document.write('<!doctype html><html><head></head><body></body></html>')
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLInputElement',
  'SVGElement',
  'Node',
  'Element',
  'Event',
  'MouseEvent',
  'CustomEvent',
  'MutationObserver',
  'ResizeObserver',
  'requestAnimationFrame',
  'cancelAnimationFrame',
  'getComputedStyle',
  'localStorage',
  'sessionStorage',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: dom[key],
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
const { AxiosError } = await import('axios')
const { toast } = await import('sonner')
const { createQueryRetry } = await import('@/lib/query-retry')
const { PublicScriptsPanel, ScriptsAdminSection } =
  await import('./scripts-panel')
const { useAuthStore } = await import('@/stores/auth-store')
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
after(() => dom.close())
const flush = () => new Promise((resolve) => setTimeout(resolve, 30))

test('repository edits preserve the read revision and pull only the saved configuration', async () => {
  const original = api.defaults.adapter
  const calls: { method: string | undefined; data: unknown }[] = []
  let revision = 4
  let branch = 'main'
  api.defaults.adapter = async (config) => {
    let data: unknown
    if (config.url === '/api/scripts') {
      data = { success: true, data: [] }
    } else {
      assert.ok(config.url?.startsWith('/api/scripts/repository'))
      if (config.method !== 'get') {
        const body = config.data ? JSON.parse(String(config.data)) : null
        calls.push({ method: config.method, data: body })
        if (config.method === 'put') {
          assert.equal(
            body.revision,
            4,
            'keep the revision that the draft started from'
          )
          branch = body.branch
        }
        revision++
      }
      data = {
        success: true,
        data: {
          revision,
          repository_url: 'https://github.com/example/scripts',
          branch,
          github_key_set: false,
        },
      }
    }
    return { config, data, headers: {}, status: 200, statusText: 'OK' }
  }
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const container = document.body.appendChild(document.createElement('div'))
  const root = createRoot(container)
  const button = (label: string) => {
    const found = [...container.querySelectorAll('button')].find(
      (node) => node.textContent?.trim() === label
    )
    assert.ok(found, label)
    return found
  }
  try {
    await act(async () => {
      root.render(
        <I18nextProvider i18n={i18n}>
          <QueryClientProvider client={client}>
            <ScriptsAdminSection />
          </QueryClientProvider>
        </I18nextProvider>
      )
      await flush()
    })
    await act(async () => {
      await flush()
    })
    assert.equal(button('Pull updates').disabled, false)
    const input = container.querySelector<HTMLInputElement>(
      '#scripts-repository-branch'
    )
    assert.ok(input)
    const credential = container.querySelector<HTMLInputElement>(
      '#scripts-github-key'
    )
    assert.ok(input.form)
    assert.equal(credential?.form, input.form)
    assert.equal(button('Save repository').type, 'submit')
    assert.equal(button('Pull updates').type, 'button')
    const setter = Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      'value'
    )?.set
    assert.ok(setter)
    await act(async () => {
      setter.call(input, 'release')
      input.dispatchEvent(new Event('input', { bubbles: true }))
      await flush()
    })
    assert.equal(button('Pull updates').disabled, true)
    // A background refresh must not silently approve overwriting another admin.
    revision = 5
    await act(async () => {
      await client.invalidateQueries({ queryKey: ['scripts-repository'] })
      await flush()
    })
    await act(async () => {
      button('Save repository').click()
      await flush()
    })
    await act(async () => {
      await flush()
    })
    assert.deepEqual(calls, [
      {
        method: 'put',
        data: {
          revision: 4,
          repository_url: 'https://github.com/example/scripts',
          branch: 'release',
          github_key: '',
          clear_github_key: false,
        },
      },
    ])
    assert.equal(button('Pull updates').disabled, false)
    await act(async () => {
      button('Pull updates').click()
      await flush()
    })
    assert.equal(calls.length, 2)
    assert.equal(calls[1].method, 'post')
  } finally {
    await act(async () => root.unmount())
    client.clear()
    container.remove()
    api.defaults.adapter = original
  }
})

test('private repository controls require L6 and preserve credential CAS without caching the secret', async () => {
  const original = api.defaults.adapter
  const previousAuth = useAuthStore.getState().auth
  useAuthStore.setState({
    auth: {
      ...previousAuth,
      user: { id: 901, username: 'fixture-admin', role: 10 },
    },
  })
  let revision = 4
  let keySet = true
  let conflict = false
  const writes: unknown[] = []
  const secret = 'github_pat_ui_fixture_replacement'
  api.defaults.adapter = async (config) => {
    if (config.url === '/api/scripts') {
      return {
        data: { success: true, data: [] },
        status: 200,
        statusText: 'OK',
        headers: {},
        config,
      }
    }
    if (config.method === 'put') {
      const body: unknown = JSON.parse(config.data)
      writes.push(body)
      if (conflict) {
        return {
          data: { success: false, message: 'conflict' },
          status: 200,
          statusText: 'OK',
          headers: {},
          config,
        }
      }
      assert.ok(
        typeof body === 'object' && body !== null && 'clear_github_key' in body
      )
      keySet = !body.clear_github_key
      revision += 1
    }
    return {
      data: {
        success: true,
        data: {
          revision,
          repository_url: 'https://github.com/owner/private',
          branch: 'main',
          github_key_set: keySet,
        },
      },
      status: 200,
      statusText: 'OK',
      headers: {},
      config,
    }
  }
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  })
  const container = document.body.appendChild(document.createElement('div'))
  const root = createRoot(container)
  const button = (label: string) => {
    const result = [...container.querySelectorAll('button')].find(
      (node) => node.textContent?.trim() === label
    )
    assert.ok(result, label)
    return result
  }
  const input = () => {
    const result = container.querySelector<HTMLInputElement>(
      '#scripts-github-key'
    )
    assert.ok(result)
    return result
  }
  const typeKey = async () =>
    act(async () => {
      const setter = Object.getOwnPropertyDescriptor(
        HTMLInputElement.prototype,
        'value'
      )?.set
      assert.ok(setter)
      setter.call(input(), secret)
      input().dispatchEvent(new Event('input', { bubbles: true }))
      await flush()
    })
  try {
    await act(async () => {
      root.render(
        <I18nextProvider i18n={i18n}>
          <QueryClientProvider client={client}>
            <ScriptsAdminSection />
          </QueryClientProvider>
        </I18nextProvider>
      )
      await flush()
    })
    await act(async () => {
      await flush()
    })
    assert.equal(input().disabled, true)
    assert.equal(button('Save repository').disabled, true)
    assert.equal(button('Pull updates').disabled, true)
    await act(async () => {
      useAuthStore.setState({
        auth: {
          ...previousAuth,
          user: { id: 901, username: 'fixture-root', role: 100 },
        },
      })
      await flush()
    })
    assert.equal(input().disabled, false)
    await typeKey()
    assert.equal(button('Pull updates').disabled, true)
    await act(async () => {
      button('Save repository').click()
      await flush()
    })
    await act(async () => {
      await flush()
    })
    assert.deepEqual(writes[0], {
      revision: 4,
      repository_url: 'https://github.com/owner/private',
      branch: 'main',
      github_key: secret,
      clear_github_key: false,
    })
    assert.equal(input().value, '')
    assert.equal(
      input().placeholder,
      'Key is configured; leave blank to keep it'
    )
    const clear = container.querySelector<HTMLInputElement>(
      'input[type=checkbox]'
    )
    assert.ok(clear)
    await typeKey()
    await act(async () => {
      clear.click()
      await flush()
    })
    assert.equal(input().value, '')
    await act(async () => {
      button('Save repository').click()
      await flush()
    })
    await act(async () => {
      await flush()
    })
    assert.deepEqual(writes[1], {
      revision: 5,
      repository_url: 'https://github.com/owner/private',
      branch: 'main',
      github_key: '',
      clear_github_key: true,
    })
    assert.equal(container.querySelector('input[type=checkbox]'), null)
    await typeKey()
    revision = 7
    conflict = true
    await act(async () => {
      await client.invalidateQueries({ queryKey: ['scripts-repository'] })
      await flush()
    })
    await act(async () => {
      button('Save repository').click()
      await flush()
    })
    assert.deepEqual(writes[2], {
      revision: 6,
      repository_url: 'https://github.com/owner/private',
      branch: 'main',
      github_key: secret,
      clear_github_key: false,
    })
    assert.equal(button('Pull updates').disabled, true)
    assert.equal(
      JSON.stringify(
        client
          .getQueryCache()
          .getAll()
          .map((query) => query.state.data)
      ).includes(secret),
      false
    )
    assert.equal(
      JSON.stringify(
        client
          .getMutationCache()
          .getAll()
          .map((mutation) => mutation.state)
      ).includes(secret),
      false
    )
  } finally {
    await act(async () => root.unmount())
    client.clear()
    container.remove()
    api.defaults.adapter = original
    useAuthStore.setState({ auth: previousAuth })
  }
})

test('the embedded and admin scripts panels share a 503 and its explicit retry', async () => {
  const originalAdapter = api.defaults.adapter
  const originalToast = toast.error
  const toasts: unknown[] = []
  let available = false
  let listRequests = 0
  toast.error = ((message: unknown) => {
    toasts.push(message)
    return 'scripts-test-error'
  }) as typeof toast.error
  api.defaults.adapter = async (config) => {
    let data: unknown
    if (config.url === '/api/scripts') {
      listRequests++
      if (!available) {
        throw new AxiosError(
          'Request failed with status code 503',
          AxiosError.ERR_BAD_RESPONSE,
          config,
          undefined,
          {
            config,
            status: 503,
            statusText: 'Service Unavailable',
            headers: {},
            data: {
              error: {
                code: 'core_migration_incomplete',
                message: 'The scripts catalog is not enabled.',
              },
            },
          }
        )
      }
      data = { success: true, data: [{ name: 'menu.sh' }] }
    } else if (config.url === '/api/scripts/repository') {
      data = {
        success: true,
        data: {
          revision: 1,
          repository_url: '',
          branch: 'main',
          github_key_set: false,
        },
      }
    } else if (config.url === '/api/site/navigation') {
      data = {
        success: true,
        message: '',
        data: { value: '{}', revision: 1 },
      }
    } else {
      assert.equal(config.url, '/api/status')
      data = {
        success: true,
        data: { server_address: 'https://console.example.test' },
      }
    }
    return { config, data, headers: {}, status: 200, statusText: 'OK' }
  }
  // Use the production retry policy so this checks the panel's own override.
  const client = new QueryClient({
    defaultOptions: {
      queries: {
        retry: createQueryRetry(true),
        retryDelay: 1,
        refetchOnWindowFocus: false,
      },
    },
  })
  const container = document.body.appendChild(document.createElement('div'))
  const root = createRoot(container)
  try {
    await act(async () => {
      root.render(
        <I18nextProvider i18n={i18n}>
          <QueryClientProvider client={client}>
            <PublicScriptsPanel />
            <ScriptsAdminSection />
          </QueryClientProvider>
        </I18nextProvider>
      )
      await flush()
    })
    await act(flush)
    assert.equal(listRequests, 1)
    assert.equal(client.getQueryState(['public-scripts'])?.status, 'error')
    assert.equal(client.getQueryData(['public-scripts']), undefined)
    const alerts = container.querySelectorAll('[role="alert"]')
    assert.equal(alerts.length, 2)
    for (const alert of alerts) {
      assert.match(alert.textContent ?? '', /Unable to load scripts/)
    }
    assert.doesNotMatch(
      container.textContent ?? '',
      /Pull a repository to publish scripts/
    )
    assert.deepEqual(toasts, [])

    const retry = alerts[1].querySelector('button')
    assert.ok(retry)
    available = true
    await act(async () => {
      retry.click()
      await flush()
    })
    await act(flush)
    assert.equal(listRequests, 2)
    assert.equal(client.getQueryState(['public-scripts'])?.status, 'success')
    assert.equal(container.querySelector('[role="alert"]'), null)
    assert.match(container.textContent ?? '', /menu\.sh/)
    assert.deepEqual(toasts, [])
  } finally {
    await act(async () => root.unmount())
    client.clear()
    container.remove()
    api.defaults.adapter = originalAdapter
    toast.error = originalToast
  }
})
