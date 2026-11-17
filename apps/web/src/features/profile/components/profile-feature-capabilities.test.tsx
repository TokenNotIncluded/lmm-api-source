/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { Window } from 'happy-dom'

const domWindow = new Window({ url: 'https://console.example.test/profile' })
// These tests cover capability/error behavior with a reduced-motion user preference.
const matchMedia = domWindow.matchMedia.bind(domWindow)
Object.defineProperty(domWindow, 'matchMedia', {
  value: (query: string) => {
    const media = matchMedia(query)
    if (query.includes('prefers-reduced-motion')) {
      Object.defineProperty(media, 'matches', { value: true })
    }
    return media
  },
})
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'SVGElement',
  'Node',
  'Element',
  'Event',
  'MouseEvent',
  'MutationObserver',
  'ResizeObserver',
  'requestAnimationFrame',
  'cancelAnimationFrame',
  'getComputedStyle',
  'localStorage',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: domWindow[key],
  })
}
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { ProfileTwoFactorCapability, ProfileCheckinCapability } =
  await import('./profile-feature-capabilities')
const { GiftCard } = await import('./gift-card')
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
const previousGet = api.get
const previousAuth = useAuthStore.getState().auth
let mounted:
  | {
      root: ReturnType<typeof createRoot>
      host: HTMLDivElement
      client: InstanceType<typeof QueryClient>
    }
  | undefined

afterEach(async () => {
  if (mounted) {
    await act(async () => mounted?.root.unmount())
    mounted.client.clear()
    mounted.host.remove()
    mounted = undefined
  }
  api.get = previousGet
  useAuthStore.setState({ auth: previousAuth })
})
after(() => domWindow.close())

async function render(content: import('react').ReactNode) {
  if (!mounted) {
    const host = document.createElement('div')
    document.body.append(host)
    mounted = {
      host,
      root: createRoot(host),
      client: new QueryClient({
        defaultOptions: { queries: { retry: false } },
      }),
    }
  }
  const { root, client, host } = mounted
  await act(async () => {
    root.render(
      <QueryClientProvider client={client}>
        <I18nextProvider i18n={i18n}>{content}</I18nextProvider>
      </QueryClientProvider>
    )
    await new Promise((resolve) => setTimeout(resolve, 30))
  })
  return host
}

test('unverified and disabled public capabilities do not call MFA or check-in handlers', async () => {
  const urls: string[] = []
  api.get = (async (url) => {
    urls.push(url)
    throw new Error('Unexpected disabled feature read')
  }) as typeof api.get
  for (const ready of [false, true]) {
    const host = await render(
      <>
        <ProfileTwoFactorCapability
          ready={ready}
          enabled={false}
          loading={false}
          onRefresh={() => {}}
        />
        <ProfileCheckinCapability
          ready={ready}
          enabled={false}
          loading={false}
          onRefresh={() => {}}
        />
      </>
    )
    assert.match(
      host.textContent ?? '',
      ready
        ? /not available on this server/
        : /capabilities have not been loaded/
    )
  }
  assert.deepEqual(urls, [])
})

test('an enabled MFA capability reads the actual native status contract', async () => {
  const urls: string[] = []
  api.get = (async (url, config) => {
    urls.push(url)
    assert.equal(config?.skipErrorHandler, true)
    return {
      data: {
        success: true,
        data: { enabled: false, locked: false, backup_codes_remaining: 0 },
      },
    }
  }) as typeof api.get
  const host = await render(
    <ProfileTwoFactorCapability
      ready
      enabled
      loading={false}
      onRefresh={() => {}}
    />
  )
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 30))
  })
  assert.deepEqual(urls, ['/api/user/2fa/status'])
  assert.match(host.textContent ?? '', /Two-Step Verification/)
})

test('enabled but failing check-in remains an error with explicit retry', async () => {
  let calls = 0
  api.get = (async (url) => {
    assert.equal(url, '/api/checkin/capabilities')
    calls++
    throw new Error('Request failed with status code 503')
  }) as typeof api.get
  const host = await render(
    <ProfileCheckinCapability
      ready
      enabled
      loading={false}
      onRefresh={() => {}}
    />
  )
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 30))
  })
  assert.match(host.textContent ?? '', /Unable to load check-in availability/)
  assert.equal(calls, 1)
  const retry = [...host.querySelectorAll('button')].find(
    (button) => button.textContent === 'Retry'
  )
  assert.ok(retry)
  await act(async () => {
    retry.click()
    await new Promise((resolve) => setTimeout(resolve, 30))
  })
  assert.equal(calls, 2)
  assert.match(host.textContent ?? '', /Unable to load check-in availability/)
})

test('gift read failures are not an empty gift list or repeated automatic claims', async () => {
  useAuthStore.setState({
    auth: {
      ...previousAuth,
      user: { id: 7, role: 1 },
      session: { sid: 'synthetic-profile-session' },
    } as never,
  })
  let calls = 0
  api.get = (async (url, config) => {
    assert.equal(url, '/api/user/gift')
    assert.equal(config?.skipErrorHandler, true)
    assert.deepEqual(config?.authScope, {
      userId: 7,
      sessionId: 'synthetic-profile-session',
    })
    calls++
    return {
      data: { success: false, message: 'Compensation service unavailable' },
    }
  }) as typeof api.get
  const host = await render(<GiftCard />)
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 30))
  })
  assert.equal(calls, 1)
  assert.match(host.textContent ?? '', /Unable to load compensation gifts/)
  assert.match(host.textContent ?? '', /Compensation service unavailable/)
  const retry = [...host.querySelectorAll('button')].find(
    (button) => button.textContent === 'Retry'
  )
  assert.ok(retry)
  await act(async () => {
    retry.click()
    await new Promise((resolve) => setTimeout(resolve, 30))
  })
  assert.equal(calls, 2)
})
