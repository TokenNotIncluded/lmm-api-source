/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.

For commercial licensing, please contact support@quantumnous.com
*/
import assert from 'node:assert/strict'
import { after, afterEach, describe, test } from 'node:test'

import { Window } from 'happy-dom'

const domWindow = new Window()
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'SVGElement',
  'Node',
  'Element',
  'Event',
  'CustomEvent',
  'MutationObserver',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: domWindow[key],
  })
}

const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { ChallengeList } = await import('@/features/forge/challenge-list')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { useNotifications } = await import('./use-notifications')

const originalGet = api.get
const originalPost = api.post
const reactTestGlobals = globalThis as typeof globalThis & {
  IS_REACT_ACT_ENVIRONMENT?: boolean
}
reactTestGlobals.IS_REACT_ACT_ENVIRONMENT = true

const i18n = createInstance()
await i18n.use(initReactI18next).init({
  lng: 'en',
  resources: { en: { translation: {} } },
})

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((nextResolve) => {
    resolve = nextResolve
  })
  return { promise, resolve }
}

async function flushQueries() {
  await new Promise((resolve) => setTimeout(resolve, 10))
}

function cachedNewBackendStatus() {
  return {
    backend_capabilities: {
      bounty_notifications: true,
      bounty_challenge_cancel: true,
      bounty_public_read: true,
      self_oauth_unbind: true,
      responses_websocket: true,
    },
  }
}

let latestNotifications: ReturnType<typeof useNotifications> | null = null

function NotificationsProbe() {
  latestNotifications = useNotifications()
  return null
}

afterEach(() => {
  api.get = originalGet
  api.post = originalPost
  useAuthStore.getState().auth.reset('complete')
  window.localStorage.clear()
  latestNotifications = null
})

after(() => domWindow.close())

describe('backend capability query safety', () => {
  test('does not call unified or legacy bounty notifications without a live capability', async () => {
    useAuthStore.getState().auth.setUser({
      id: 7,
      username: 'compat-user',
      role: 1,
    })
    const statusResponse = deferred<{
      data: { success: boolean; data: { version: string } }
    }>()
    const gets: string[] = []
    const posts: string[] = []
    api.get = (async (url) => {
      gets.push(url)
      if (url === '/api/status') return statusResponse.promise
      if (url === '/api/notice') {
        return { data: { success: true, data: '' } }
      }
      if (url === '/api/open-source-bounties/tips/received') {
        return {
          data: {
            success: true,
            data: [
              {
                id: 1,
                project_id: 2,
                challenge_id: 3,
                sender_user_id: 4,
                sender_username: 'legacy-sender',
                project_title: 'Legacy project',
                quota: 100,
                note: '',
                recipient_read_at: 0,
                thanked_at: 0,
                created_at: 1,
              },
            ],
          },
        }
      }
      return { data: { success: true, data: [] } }
    }) as typeof api.get
    api.post = (async (url) => {
      posts.push(url)
      return { data: { success: true, data: null } }
    }) as typeof api.post

    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    })
    queryClient.setQueryData(
      ['status', 'user:7:docs:0'],
      cachedNewBackendStatus()
    )
    queryClient.setQueryData(
      ['open-source-bounties', 'notifications', 'unified', 7],
      [
        {
          id: 1,
          kind: 'tip_transfer',
          recipient_read_at: 0,
        },
      ]
    )
    const container = document.createElement('div')
    const root = createRoot(container)

    await act(async () => {
      root.render(
        <QueryClientProvider client={queryClient}>
          <I18nextProvider i18n={i18n}>
            <NotificationsProbe />
          </I18nextProvider>
        </QueryClientProvider>
      )
      await flushQueries()
    })

    assert.equal(
      gets.includes('/api/open-source-bounties/notifications'),
      false
    )

    await act(async () => {
      statusResponse.resolve({
        data: { success: true, data: { version: 'legacy-go' } },
      })
      await flushQueries()
    })

    assert.equal(
      gets.includes('/api/open-source-bounties/tips/received'),
      false
    )
    assert.equal(
      gets.includes('/api/open-source-bounties/notifications'),
      false
    )

    await act(async () => {
      latestNotifications?.openPopover('bounty-tips')
      await latestNotifications?.thankTip(1)
      await flushQueries()
    })
    assert.equal(
      posts.includes('/api/open-source-bounties/tips/received/read'),
      false
    )
    assert.equal(
      posts.includes('/api/open-source-bounties/notifications/read'),
      false
    )
    assert.equal(
      posts.some((url) => url.includes('/tips/1/thank')),
      false
    )

    await act(async () => root.unmount())
    queryClient.clear()
  })

  test('does not query bounty notifications before developer access activation', async () => {
    useAuthStore.getState().auth.setUser({
      id: 8,
      username: 'pending-user',
      role: 1,
      developer_access_granted: false,
    })
    const gets: string[] = []
    api.get = (async (url) => {
      gets.push(url)
      if (url === '/api/status') {
        return {
          data: {
            success: true,
            data: { version: 'go', ...cachedNewBackendStatus() },
          },
        }
      }
      if (url === '/api/notice') return { data: { success: true, data: '' } }
      return { data: { success: true, data: [] } }
    }) as typeof api.get

    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    })
    const container = document.createElement('div')
    const root = createRoot(container)

    await act(async () => {
      root.render(
        <QueryClientProvider client={queryClient}>
          <I18nextProvider i18n={i18n}>
            <NotificationsProbe />
          </I18nextProvider>
        </QueryClientProvider>
      )
      await flushQueries()
    })

    assert.equal(
      gets.includes('/api/open-source-bounties/notifications'),
      false
    )

    await act(async () => root.unmount())
    queryClient.clear()
  })

  test('does not issue a public bounty request for signed-in users from cached capabilities', async () => {
    useAuthStore.getState().auth.setUser({
      id: 9,
      username: 'cached-user',
      role: 1,
    })
    const statusResponse = deferred<{
      data: { success: boolean; data: { version: string } }
    }>()
    const gets: string[] = []
    api.get = (async (url) => {
      gets.push(url)
      if (url === '/api/status') return statusResponse.promise
      return { data: { success: true, data: [] } }
    }) as typeof api.get

    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    })
    queryClient.setQueryData(
      ['status', 'user:9:docs:0'],
      cachedNewBackendStatus()
    )
    const container = document.createElement('div')
    const root = createRoot(container)

    await act(async () => {
      root.render(
        <QueryClientProvider client={queryClient}>
          <I18nextProvider i18n={i18n}>
            <ChallengeList />
          </I18nextProvider>
        </QueryClientProvider>
      )
      await flushQueries()
    })

    statusResponse.resolve({
      data: { success: true, data: { version: 'legacy-go' } },
    })
    await act(flushQueries)

    assert.equal(
      gets.some((url) => url.startsWith('/api/open-source-bounties?')),
      false
    )

    await act(async () => root.unmount())
    queryClient.clear()
  })
})
