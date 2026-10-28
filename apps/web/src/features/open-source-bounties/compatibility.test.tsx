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
domWindow.document.write(
  '<!doctype html><html><head></head><body></body></html>'
)
Object.defineProperty(domWindow.document, 'compatMode', {
  configurable: true,
  value: 'CSS1Compat',
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
  'CustomEvent',
  'MutationObserver',
  'requestAnimationFrame',
  'cancelAnimationFrame',
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
const { ChallengeCancelAction } = await import('./index')
const { api } = await import('@/lib/api')

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

const challenge = {
  id: 71,
  project_id: 41,
  participant_user_id: 2,
  participant_username: 'contributor',
  github_handle: 'contributor',
  status: 'accepted' as const,
  issue_url: '',
  pull_request_url: '',
  submission_note: '',
  review_note: '',
  reward_quota: 100,
  tip_quota: 0,
  owner_rating_score: 0,
  owner_rating_comment: '',
  owner_rated_at: 0,
  contributor_rating_score: 0,
  contributor_rating_comment: '',
  contributor_rated_at: 0,
  accepted_at: 0,
  submitted_at: 0,
  reviewed_at: 0,
  paid_at: 0,
  participant_rating_average: 0,
  participant_rating_count: 0,
  owner_rating_average: 0,
  owner_rating_count: 0,
  owner_thank_heart_count: 0,
}

function cachedStatus() {
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

async function renderAction(queryClient: InstanceType<typeof QueryClient>) {
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  await act(async () => {
    root.render(
      <QueryClientProvider client={queryClient}>
        <I18nextProvider i18n={i18n}>
          <ChallengeCancelAction
            challenge={challenge}
            pending=''
            onCancel={(challenge) => {
              void api.post(
                `/api/open-source-bounties/challenges/${challenge.id}/cancel`
              )
            }}
          />
        </I18nextProvider>
      </QueryClientProvider>
    )
    await flushQueries()
  })
  return { container, root }
}

function cancelButton() {
  return [...document.querySelectorAll('button')].find(
    (button) => button.textContent?.trim() === 'Cancel challenge'
  ) as HTMLButtonElement | undefined
}

afterEach(() => {
  api.get = originalGet
  api.post = originalPost
  window.localStorage.clear()
  document.body.replaceChildren()
})

after(() => domWindow.close())

describe('legacy Go challenge action compatibility', () => {
  test('does not render or call cancellation from stale capabilities after legacy status', async () => {
    const statusResponse = deferred<{
      data: { success: boolean; data: { version: string } }
    }>()
    const posts: string[] = []
    api.get = (async (url) => {
      if (url === '/api/status') return statusResponse.promise
      return { data: { success: true, data: [] } }
    }) as typeof api.get
    api.post = (async (url) => {
      posts.push(url)
      return { data: { success: true, data: null } }
    }) as typeof api.post
    window.localStorage.setItem('status', JSON.stringify(cachedStatus()))

    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    })
    queryClient.setQueryData(['status', 'anonymous'], cachedStatus())
    const { root } = await renderAction(queryClient)

    assert.equal(cancelButton(), undefined)
    statusResponse.resolve({
      data: { success: true, data: { version: 'legacy-go' } },
    })
    await act(flushQueries)

    assert.equal(cancelButton(), undefined)
    assert.deepEqual(posts, [])

    await act(async () => root.unmount())
    queryClient.clear()
  })

  test('renders and calls cancellation when live status advertises the capability', async () => {
    const statusResponse = deferred<{
      data: { success: boolean; data: ReturnType<typeof cachedStatus> }
    }>()
    const posts: string[] = []
    api.get = (async (url) => {
      if (url === '/api/status') return statusResponse.promise
      return { data: { success: true, data: [] } }
    }) as typeof api.get
    api.post = (async (url) => {
      posts.push(url)
      return { data: { success: true, data: null } }
    }) as typeof api.post

    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    })
    queryClient.setQueryData(['status', 'anonymous'], cachedStatus())
    const { root } = await renderAction(queryClient)
    assert.equal(cancelButton(), undefined)

    statusResponse.resolve({
      data: { success: true, data: cachedStatus() },
    })
    await act(flushQueries)

    const button = cancelButton()
    assert.ok(button)
    await act(async () => button.click())
    await flushQueries()
    assert.deepEqual(posts, ['/api/open-source-bounties/challenges/71/cancel'])

    await act(async () => root.unmount())
    queryClient.clear()
  })
})
