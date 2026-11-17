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

import type { AuthUser } from '@/stores/auth-store'

const domWindow = new Window({ url: 'https://console.example.test/guide' })
Object.defineProperty(domWindow.document, 'compatMode', { value: 'CSS1Compat' })
for (const key of [
  'window',
  'document',
  'navigator',
  'history',
  'location',
  'HTMLElement',
  'HTMLAnchorElement',
  'HTMLButtonElement',
  'HTMLInputElement',
  'SVGElement',
  'Node',
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
  'scrollTo',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: domWindow[key],
  })
}
Object.defineProperty(domWindow.HTMLElement.prototype, 'scrollIntoView', {
  configurable: true,
  value() {},
})
const matchMediaStub = () => ({
  matches: false,
  media: '',
  addListener() {},
  removeListener() {},
  addEventListener() {},
  removeEventListener() {},
  dispatchEvent() {
    return false
  },
})
Object.defineProperty(domWindow, 'matchMedia', {
  configurable: true,
  value: matchMediaStub,
})
Object.defineProperty(globalThis, 'matchMedia', {
  configurable: true,
  value: matchMediaStub,
})
Object.defineProperty(globalThis, 'customElements', {
  configurable: true,
  value: {
    get() {
      return undefined
    },
    define() {},
  },
})

const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const {
  Outlet,
  RouterProvider,
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
} = await import('@tanstack/react-router')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { consumeQueuedAssistantRequest } =
  await import('@/features/assistant/assistant-events')
const { useAuthStore } = await import('@/stores/auth-store')
const { Guide } = await import('./index')

const originalGet = api.get
const reactTestGlobals = globalThis as typeof globalThis & {
  IS_REACT_ACT_ENVIRONMENT?: boolean
}
reactTestGlobals.IS_REACT_ACT_ENVIRONMENT = true
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
let dispose: (() => Promise<void>) | undefined

async function flushEffects() {
  await new Promise((resolve) => setTimeout(resolve, 20))
}

async function renderGuide(user: AuthUser | null, serverAddress?: string) {
  useAuthStore.getState().auth.setUser(user)
  api.get = (async (url) => {
    if (url === '/api/status') {
      return {
        data: {
          success: true,
          data: {
            assistant: { enabled: false },
            server_address: serverAddress,
            data: { server_address: 'https://nested.example.test' },
            system_name: 'Custom Relay',
          },
        },
      }
    }
    return { data: { success: true, data: url === '/api/notice' ? '' : [] } }
  }) as typeof api.get
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const rootRoute = createRootRoute({ component: Outlet })
  const guideRoute = createRoute({
    getParentRoute: () => rootRoute,
    path: '/guide',
    component: Guide,
  })
  const destinations = [
    '/sign-in',
    '/wallet',
    '/keys',
    '/support',
    '/pricing',
  ].map((path) =>
    createRoute({
      getParentRoute: () => rootRoute,
      path,
      component: () => null,
    })
  )
  const router = createRouter({
    routeTree: rootRoute.addChildren([guideRoute, ...destinations]),
    history: createMemoryHistory({ initialEntries: ['/guide'] }),
  })
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  dispose = async () => {
    await act(async () => root.unmount())
    queryClient.clear()
    container.remove()
  }
  await act(async () => {
    root.render(
      <QueryClientProvider client={queryClient}>
        <I18nextProvider i18n={i18n}>
          <RouterProvider router={router} />
        </I18nextProvider>
      </QueryClientProvider>
    )
    await flushEffects()
  })
  await act(flushEffects)
  return { container, router }
}

function findButton(container: HTMLElement, label: string) {
  const button = [
    ...container.querySelectorAll<HTMLButtonElement>('button'),
  ].find((item) => item.textContent?.trim() === label)
  assert.ok(button, `Missing button: ${label}`)
  return button
}

async function click(button: HTMLButtonElement) {
  await act(async () => {
    button.click()
    await flushEffects()
  })
}

afterEach(async () => {
  await dispose?.()
  dispose = undefined
  consumeQueuedAssistantRequest()
  api.get = originalGet
  useAuthStore.getState().auth.reset('complete')
  window.localStorage.clear()
  window.sessionStorage.clear()
  document.body.replaceChildren()
})
after(() => domWindow.close())

describe('Guide when the AI assistant is disabled', () => {
  test('opens on client configuration with one optional contents list and all setup stages', async () => {
    const { container } = await renderGuide(null)
    const overview =
      container.querySelector<HTMLDetailsElement>('#guide-overview')
    assert.ok(overview)
    assert.equal(overview.open, false)
    assert.equal(
      overview.querySelector('summary')?.textContent?.trim(),
      'On this page'
    )
    assert.equal(
      container.querySelector('.guide-workspace-header h1')?.textContent,
      'Client configuration'
    )
    assert.equal(container.querySelectorAll('.guide-contents').length, 1)
    assert.equal(
      container.querySelector('.guide-content > section')?.id,
      'client-setup'
    )
    assert.equal(
      container.querySelector('#client-setup')?.closest('details'),
      null
    )
    assert.ok(container.querySelector('#client-setup button'))

    await act(async () => {
      overview.querySelector('summary')?.click()
      await flushEffects()
    })
    assert.equal(overview.open, true)
    assert.equal(overview.querySelectorAll(':scope > ol > li').length, 5)
    const contents = overview.querySelector('nav')
    assert.ok(contents)
    const links = [...contents.querySelectorAll<HTMLAnchorElement>('a')]
    assert.equal(links.length, 4)
    for (const link of links) {
      assert.ok(container.querySelector(link.hash), link.textContent ?? '')
    }
    assert.equal(consumeQueuedAssistantRequest(), undefined)
  })

  test('keeps public installation available and returns visitors to the guide after sign-in', async () => {
    const { container, router } = await renderGuide(null)
    assert.doesNotMatch(
      container.textContent ?? '',
      /Sign in for AI guidance|Walk me through this with the AI assistant/
    )
    assert.ok(
      container.querySelector('#client-setup a[target="_blank"]'),
      'Official downloads remain accessible'
    )
    await click(findButton(container, 'Pi (OAuth)'))
    assert.ok(container.querySelector('#pi-oauth'))
    assert.doesNotMatch(
      container.textContent ?? '',
      /Create a key and import|Create an API key/
    )

    assert.match(
      container.textContent ?? '',
      /lmm-pi-provider npm:@tokennotincluded\/pi-lmm-provider@latest/
    )
    assert.ok(
      container.querySelector(
        '#pi-oauth a[href*="npmjs.com/package/@earendil-works/pi-coding-agent"]'
      )
    )

    await click(findButton(container, 'DSH Desktop (OAuth)'))
    assert.ok(container.querySelector('#dsh-oauth'))
    assert.match(
      container.textContent ?? '',
      /@tokennotincluded\/dsh-lmm-provider@latest/
    )
    assert.match(
      container.textContent ?? '',
      /dsh plugin --profile web add "\$\(npm view @tokennotincluded\/dsh-lmm-provider@latest dist\.tarball --prefer-online\)"/
    )
    assert.match(
      container.textContent ?? '',
      /npm pack @tokennotincluded\/dsh-lmm-provider@latest/
    )
    assert.doesNotMatch(
      container.textContent ?? '',
      /Create a key and import|Create an API key/
    )

    await click(findButton(container, 'Codewhale (OAuth)'))
    assert.ok(container.querySelector('#codewhale-oauth'))
    assert.match(
      container.textContent ?? '',
      /git clone https:\/\/github\.com\/TokenNotIncluded\/codewhale-lmm-provider\.git/
    )
    assert.match(container.textContent ?? '', /codewhale-lmm login/)
    assert.match(container.textContent ?? '', /codewhale-lmm models/)
    assert.match(
      container.textContent ?? '',
      /codewhale-lmm run --model '<FULL_MODEL_ID>'/
    )
    assert.doesNotMatch(
      container.textContent ?? '',
      /Create a key and import|Create an API key/
    )

    await click(findButton(container, 'Cherry Studio'))
    const signIn = [
      ...container.querySelectorAll<HTMLAnchorElement>('a'),
    ].find((link) => link.textContent?.trim() === 'Sign in to continue setup')
    assert.ok(signIn, 'The account setup action is a sign-in link')
    assert.equal(signIn.getAttribute('href'), '/sign-in?redirect=%2Fguide')
    await act(async () => {
      signIn.click()
      await flushEffects()
    })

    assert.equal(router.state.location.pathname, '/sign-in')
    assert.deepEqual(
      { ...router.state.location.search },
      { redirect: '/guide' }
    )
    assert.equal(consumeQueuedAssistantRequest(), undefined)
  })

  test('takes signed-in users directly to API Keys', async () => {
    const { container, router } = await renderGuide(JSON.parse(JSON.stringify({
      id: 31,
      username: 'approved',
      role: 1,
      developer_access_granted: true,
    })))

    await click(findButton(container, 'Create a key and import'))

    assert.equal(router.state.location.pathname, '/keys')
    assert.equal(consumeQueuedAssistantRequest(), undefined)
  })

  test('lets an ordinary account create a key despite stale access metadata', async () => {
    const { container, router } = await renderGuide(JSON.parse(JSON.stringify({
      id: 32,
      username: 'ordinary-user',
      role: 1,
      developer_access_granted: false,
    })))
    assert.doesNotMatch(container.textContent ?? '', /access is approved|Request API access|L1/)
    await click(findButton(container, 'Create a key and import'))
    assert.equal(router.state.location.pathname, '/keys')
    assert.equal(consumeQueuedAssistantRequest(), undefined)
  })
})

test('configured guide address retains the public prefix in client setup and copied requests', async () => {
  const { container } = await renderGuide(
    null,
    'https://relay.example.test:8443/gateway/v1'
  )
  assert.match(
    container.textContent ?? '',
    /https:\/\/relay\.example\.test:8443\/gateway/
  )
  assert.match(container.textContent ?? '', /Custom Relay/)
  assert.doesNotMatch(container.textContent ?? '', /gateway\/v1\/v1/)
  const endpoint = [...container.querySelectorAll('.guide-code')].find((node) =>
    node.textContent?.includes('API endpoint root')
  )
  assert.ok(
    endpoint?.textContent?.includes('https://relay.example.test:8443/gateway')
  )
})

test('invalid configured guide address offers no endpoint copy or generated client config', async () => {
  const { container } = await renderGuide(
    null,
    'https://secret:password@relay.example.test'
  )
  assert.match(container.textContent ?? '', /API address unavailable/)
  assert.doesNotMatch(container.textContent ?? '', /secret:password/)
  const endpoint = [...container.querySelectorAll('.guide-code')].find((node) =>
    node.textContent?.includes('API endpoint root')
  )
  assert.equal(endpoint?.querySelector('button')?.disabled, true)
})
