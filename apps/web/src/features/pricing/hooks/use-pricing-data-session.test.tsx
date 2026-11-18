/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import { after, afterEach, test } from 'node:test'

import { AxiosError, type InternalAxiosRequestConfig } from 'axios'
import { Window } from 'happy-dom'

import type { NativePricingDocument } from '../catalog-contract'

const { mock } = createRequire(import.meta.url)('bun:test') as {
  mock: { module: (name: string, factory: () => unknown) => void }
}

const domWindow = new Window({ url: 'https://console.example.test/' })
// Match other motion fixtures: happy-dom rejects canceled native animations.
Object.defineProperty(domWindow.HTMLElement.prototype, 'animate', {
  configurable: true,
  value: undefined,
})
domWindow.document.write('<!doctype html><html><body></body></html>')
Object.defineProperty(domWindow.document, 'compatMode', {
  configurable: true,
  value: 'CSS1Compat',
})
const globals = new Map<string, PropertyDescriptor | undefined>()
for (const key of [
  'window',
  'document',
  'navigator',
  'history',
  'location',
  'localStorage',
  'HTMLElement',
  'HTMLButtonElement',
  'HTMLInputElement',
  'SVGElement',
  'Node',
  'Element',
  'DocumentFragment',
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
  globals.set(key, Object.getOwnPropertyDescriptor(globalThis, key))
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: domWindow[key],
  })
}
globals.set(
  'IS_REACT_ACT_ENVIRONMENT',
  Object.getOwnPropertyDescriptor(globalThis, 'IS_REACT_ACT_ENVIRONMENT')
)
Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
  configurable: true,
  value: true,
})

const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const {
  createRouter,
  createRootRoute,
  createRoute,
  createMemoryHistory,
  RouterProvider,
} = await import('@tanstack/react-router')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { useSystemConfigStore } = await import('@/stores/system-config-store')
const { ModelPlazaProvider, useModelPlaza } =
  await import('@/context/model-plaza-provider')
const { ModelPlazaPanel } = await import('../components/model-plaza-panel')
const { usePricingData } = await import('./use-pricing-data')

mock.module('@/features/forge/forge-public-shell', () => ({
  ForgePublicShell: ({ children }: React.PropsWithChildren) => <>{children}</>,
}))
const { Pricing } = await import('../index')

const i18n = createInstance()
await i18n.use(initReactI18next).init({
  lng: 'en',
  resources: { en: { translation: {} } },
})
const originalAuth = useAuthStore.getState().auth
const originalConfig = useSystemConfigStore.getState().config
const originalAdapter = api.defaults.adapter

function setSession(userId: number, sid: string) {
  useAuthStore.getState().auth.setBundle({
    access_token: `synthetic-token-${sid}`,
    token_type: 'Bearer',
    access_expires_at: 4_000_000_000,
    user: { id: userId, username: 'synthetic', role: 1, group: 'default' },
    session: {
      sid,
      current: true,
      login_method: 'password',
      ip: null,
      user_agent: null,
      created_at: 1,
      last_active_at: 1,
      expires_at: 4_000_000_000,
    },
  })
}

function pricingDocument(
  visibility: 'public' | 'session',
  input: number
): NativePricingDocument {
  const rateBasis =
    visibility === 'public' ? 'base_group_rate' : 'effective_group_rate'
  return {
    schema_version: 1,
    visibility,
    observed_at: 100,
    credits_per_usd: 500_000,
    rate_basis: rateBasis,
    invocation_authorized: false,
    routing_version: 1,
    pricing_version: 2,
    image_pricing_version: null,
    groups: ['default'],
    models: [
      {
        id: 'default:shared-model',
        model: 'shared-model',
        group: 'default',
        apis: ['openai-completions'],
        billing_status: 'configured',
        token_price: {
          currency: 'USD',
          unit: 'million_tokens',
          rate_basis: rateBasis,
          group_multiplier: 1,
          input,
          output: input * 2,
          cache_read: null,
          cache_write: null,
          request: null,
          final_cost_depends_on_usage: true,
        },
        image_variants: [],
      },
    ],
  }
}

function deferred<T>() {
  let complete: ((value: T) => void) | undefined
  const promise = new Promise<T>((resolve) => {
    complete = resolve
  })
  return {
    promise,
    resolve(value: T) {
      assert.ok(complete)
      complete(value)
    },
  }
}

function response(config: InternalAxiosRequestConfig, data: unknown) {
  return { data, config, status: 200, statusText: 'OK', headers: {} }
}

function installAdapter(
  pricing: (
    config: InternalAxiosRequestConfig
  ) => Promise<NativePricingDocument>
) {
  api.defaults.adapter = async (config) => {
    if (config.url === '/api/status') {
      return response(config, { success: true, data: {} })
    }
    if (config.url === '/core/v1/pricing/runtime') {
      return response(config, {
        schema_version: 1,
        visibility: config.authMode === 'anonymous' ? 'public' : 'session',
        observed_at: 100,
        data: {},
      })
    }
    assert.match(config.url ?? '', /^\/core\/v1\/pricing\/(public|session)$/)
    return response(config, await pricing(config))
  }
}

const mounted: Array<{
  root: ReturnType<typeof createRoot>
  container: HTMLDivElement
  client: InstanceType<typeof QueryClient>
}> = []

async function render(component: () => React.JSX.Element) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const rootRoute = createRootRoute()
  const pricingRoute = createRoute({
    getParentRoute: () => rootRoute,
    path: '/pricing/',
    component,
  })
  const router = createRouter({
    routeTree: rootRoute.addChildren([pricingRoute]),
    history: createMemoryHistory({ initialEntries: ['/pricing/'] }),
  })
  await router.load()
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  mounted.push({ root, container, client })
  await act(async () => {
    root.render(
      <QueryClientProvider client={client}>
        <I18nextProvider i18n={i18n}>
          <RouterProvider router={router} />
        </I18nextProvider>
      </QueryClientProvider>
    )
  })
  return { client, container }
}

async function waitFor(condition: () => boolean, message: string) {
  for (let attempt = 0; attempt < 80; attempt++) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 10))
    })
    if (condition()) return
  }
  assert.fail(message)
}

afterEach(async () => {
  for (const item of mounted.splice(0)) {
    await act(async () => item.root.unmount())
    item.client.clear()
    item.container.remove()
  }
  api.defaults.adapter = originalAdapter
  useAuthStore.setState({ auth: originalAuth })
  useSystemConfigStore.getState().setConfig(originalConfig)
})

after(() => {
  domWindow.close()
  for (const [key, descriptor] of globals) {
    if (descriptor) Object.defineProperty(globalThis, key, descriptor)
    else Reflect.deleteProperty(globalThis, key)
  }
})

for (const [name, nextUserId, nextSID] of [
  ['account', 8, 'session-b'],
  ['same-account session', 7, 'session-b'],
] as const) {
  test(`pricing hides the prior ${name} rate while the new rate loads`, async () => {
    setSession(7, 'session-a')
    const next = deferred<NativePricingDocument>()
    installAdapter(async (config) =>
      config.authScope?.sessionId === 'session-a'
        ? pricingDocument('session', 71)
        : await next.promise
    )
    let latest: ReturnType<typeof usePricingData> | undefined
    function Probe() {
      latest = usePricingData()
      return <output>{latest.models[0]?.input_price ?? 'loading'}</output>
    }
    const rendered = await render(Probe)
    await waitFor(
      () => latest?.models[0]?.input_price === 71,
      'first rate missing'
    )
    await act(async () => setSession(nextUserId, nextSID))
    assert.equal(latest?.models.length, 0)
    assert.equal(latest?.catalogAvailable, false)
    assert.equal(rendered.container.textContent, 'loading')
    await act(async () => next.resolve(pricingDocument('session', 82)))
    await waitFor(
      () => latest?.models[0]?.input_price === 82,
      'new rate missing'
    )
    assert.equal(rendered.container.textContent, '82')
  })
}

test('sign-out clears the displayed account rate before public prices return', async () => {
  setSession(7, 'session-a')
  const publicPrices = deferred<NativePricingDocument>()
  installAdapter(async (config) =>
    config.authMode === 'anonymous'
      ? await publicPrices.promise
      : pricingDocument('session', 71)
  )
  let latest: ReturnType<typeof usePricingData> | undefined
  function Probe() {
    latest = usePricingData()
    return <output>{latest.models[0]?.input_price ?? 'loading'}</output>
  }
  const rendered = await render(Probe)
  await waitFor(
    () => latest?.models[0]?.input_price === 71,
    'account rate missing'
  )
  await act(async () => useAuthStore.getState().auth.reset('complete'))
  assert.equal(latest?.models.length, 0)
  assert.equal(rendered.container.textContent, 'loading')
  await act(async () => publicPrices.resolve(pricingDocument('public', 93)))
  await waitFor(
    () => latest?.models[0]?.input_price === 93,
    'public rate missing'
  )
  assert.equal(rendered.container.textContent, '93')
})

test('an unavailable refresh hides the previously verified account price', async () => {
  setSession(7, 'session-a')
  let unavailable = false
  let publicRequests = 0
  installAdapter(async (config) => {
    if (config.authMode === 'anonymous') publicRequests++
    if (unavailable) {
      throw new AxiosError(
        'Account pricing unavailable',
        undefined,
        config,
        undefined,
        {
          data: { error: 'pricing_unavailable' },
          status: 503,
          statusText: 'Service Unavailable',
          headers: {},
          config,
        }
      )
    }
    return pricingDocument('session', 71)
  })
  let latest: ReturnType<typeof usePricingData> | undefined
  function Probe() {
    latest = usePricingData()
    return (
      <output>
        {latest.error ? 'unavailable' : latest.models[0]?.input_price}
      </output>
    )
  }
  const rendered = await render(Probe)
  await waitFor(
    () => latest?.models[0]?.input_price === 71,
    'account rate missing'
  )
  unavailable = true
  await act(async () => {
    await rendered.client.refetchQueries({
      queryKey: ['pricing', 7, 'session-a'],
      exact: true,
    })
  })
  await waitFor(() => Boolean(latest?.error), 'pricing error missing')
  assert.equal(latest?.models.length, 0)
  assert.deepEqual(latest?.groupRatio, {})
  assert.equal(latest?.catalogAvailable, false)
  assert.equal(rendered.container.textContent, 'unavailable')
  assert.equal(publicRequests, 0)
})

function pricingUnavailable(config: InternalAxiosRequestConfig) {
  return new AxiosError('Pricing unavailable', undefined, config, undefined, {
    data: { error: { code: 'pricing_unavailable' } },
    status: 503,
    statusText: 'Service Unavailable',
    headers: {},
    config,
  })
}

test('the pricing page keeps unknown counts hidden and restores a verified empty catalogue after retry', async () => {
  useAuthStore.getState().auth.reset('complete')
  const initial = deferred<NativePricingDocument>()
  let requests = 0
  installAdapter(async (config) => {
    if (requests++ === 0) {
      await initial.promise
      throw pricingUnavailable(config)
    }
    return { ...pricingDocument('public', 1), models: [] }
  })
  const { container } = await render(Pricing)
  assert.doesNotMatch(
    container.textContent ?? '',
    /This site currently has|0 models/
  )
  await act(async () => initial.resolve(pricingDocument('public', 1)))
  await waitFor(
    () =>
      container.textContent?.includes('Failed to load enabled models') === true,
    'pricing failure missing'
  )
  assert.doesNotMatch(
    container.textContent ?? '',
    /This site currently has|0 models|No models found|Prices always use the base price/
  )
  assert.equal(container.querySelector('input'), null)
  const retry = [...container.querySelectorAll('button')].find(
    (button) => button.textContent?.trim() === 'Retry'
  )
  assert.ok(retry)
  await act(async () => retry.click())
  await waitFor(
    () =>
      container.textContent?.includes(
        'This site currently has 0 models enabled'
      ) === true,
    'successful empty catalogue count missing'
  )
  assert.match(container.textContent ?? '', /No models found/)
  assert.doesNotMatch(
    container.textContent ?? '',
    /Failed to load enabled models/
  )
  assert.ok(container.querySelector('input'))
  assert.equal(requests, 2)
})

test('the model panel shows counts only after the catalogue is available', async () => {
  useAuthStore.getState().auth.reset('complete')
  const initial = deferred<NativePricingDocument>()
  let requests = 0
  installAdapter(async (config) => {
    if (requests++ === 0) {
      await initial.promise
      throw pricingUnavailable(config)
    }
    return { ...pricingDocument('public', 1), models: [] }
  })
  function OpenPanel() {
    const { openPanel } = useModelPlaza()
    return (
      <>
        <button type='button' onClick={() => openPanel()}>
          Open models
        </button>
        <ModelPlazaPanel />
      </>
    )
  }
  const { container } = await render(() => (
    <ModelPlazaProvider>
      <OpenPanel />
    </ModelPlazaProvider>
  ))
  const open = container.querySelector('button')
  assert.ok(open)
  await act(async () => open.click())
  assert.equal(domWindow.document.body.textContent?.includes('0 / 0'), false)
  await act(async () => initial.resolve(pricingDocument('public', 1)))
  await waitFor(
    () =>
      domWindow.document.body.textContent?.includes(
        'Unable to load live pricing'
      ) === true,
    'panel failure missing'
  )
  assert.equal(domWindow.document.body.textContent?.includes('0 / 0'), false)
  assert.doesNotMatch(
    domWindow.document.body.textContent ?? '',
    /No models found/
  )
  const retry = [...domWindow.document.querySelectorAll('button')].find(
    (button) => button.textContent?.trim() === 'Retry'
  )
  assert.ok(retry)
  await act(async () => retry.click())
  await waitFor(
    () =>
      domWindow.document.body.textContent?.includes('0 / 0 models') === true,
    'successful empty panel count missing'
  )
  assert.match(domWindow.document.body.textContent ?? '', /No models found/)
  assert.equal(requests, 2)
})

test('an open model drawer follows the current account catalogue', async () => {
  setSession(7, 'session-a')
  const next = deferred<NativePricingDocument>()
  installAdapter(async (config) =>
    config.authScope?.sessionId === 'session-a'
      ? pricingDocument('session', 71)
      : await next.promise
  )
  function OpenPlaza() {
    const { openPanel } = useModelPlaza()
    return (
      <>
        <button type='button' onClick={() => openPanel()}>
          Open models
        </button>
        <ModelPlazaPanel />
      </>
    )
  }
  function Plaza() {
    return (
      <ModelPlazaProvider>
        <OpenPlaza />
      </ModelPlazaProvider>
    )
  }
  const rendered = await render(Plaza)
  const open = rendered.container.querySelector('button')
  assert.ok(open)
  await act(async () => open.click())
  await waitFor(
    () =>
      domWindow.document.body.textContent?.includes('shared-model') === true,
    'model row missing'
  )
  const row = [...domWindow.document.querySelectorAll('button')].find(
    (button) => button.textContent?.includes('shared-model')
  )
  assert.ok(row)
  await act(async () => row.click())
  await waitFor(
    () => domWindow.document.body.textContent?.includes('71') === true,
    'selected account model price missing'
  )
  await act(async () => setSession(8, 'session-b'))
  assert.doesNotMatch(domWindow.document.body.textContent ?? '', /\b71\b/)
  await act(async () => next.resolve(pricingDocument('session', 82)))
  await waitFor(
    () => domWindow.document.body.textContent?.includes('82') === true,
    'drawer did not update to the new account price'
  )
  assert.doesNotMatch(domWindow.document.body.textContent ?? '', /\b71\b/)
})
