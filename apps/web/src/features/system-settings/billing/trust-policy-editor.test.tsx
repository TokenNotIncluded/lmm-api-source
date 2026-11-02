/*
Copyright (C) 2026 LIghtJUNction
*/
import assert from 'node:assert/strict'
import { after, test } from 'node:test'

import { Window } from 'happy-dom'

const domWindow = new Window({
  url: 'https://console.example.test/system-settings/billing/levels-benefits',
})
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
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
Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
  configurable: true,
  value: true,
})
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const {
  RouterProvider,
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
} = await import('@tanstack/react-router')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { SettingsPageProvider } =
  await import('../components/settings-page-context')
const { TrustLevelBenefitsSection } =
  await import('./trust-level-benefits-section')

const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
after(() => domWindow.close())

const { useAuthStore } = await import('@/stores/auth-store')
const {
  assertionCredential,
  challengeReply,
  envelope,
  policyRead,
  previewReply,
  signIn,
  token,
  digest,
} = await import('./trust-policy-editor-fixtures')
const { beforeEach, afterEach } = await import('node:test')
const originalGet = api.get,
  originalPost = api.post,
  originalPut = api.put
let read = policyRead()
let writes: { path: string; body: Record<string, unknown> }[] = []
let credentialCount = 0
Object.defineProperty(domWindow, 'isSecureContext', {
  configurable: true,
  value: true,
})
Object.defineProperty(domWindow, 'PublicKeyCredential', {
  configurable: true,
  value: function PublicKeyCredential() {},
})
Object.defineProperty(domWindow.navigator, 'credentials', {
  configurable: true,
  value: {
    get: async () => {
      credentialCount++
      return assertionCredential()
    },
  },
})
beforeEach(() => {
  signIn()
  read = policyRead()
  writes = []
  credentialCount = 0
  api.get = (async (path: string) => {
    assert.equal(path, '/core/v1/admin/trust-policy')
    return envelope(read)
  }) as typeof api.get
  api.put = (async () => {
    assert.fail('Native editor must never fall back to generic PUT')
  }) as typeof api.put
  api.post = (async (path: string, body: Record<string, unknown>) => {
    writes.push({ path, body })
    if (path.endsWith('/preview')) {
      const reply = previewReply(JSON.parse(String(body.value)))
      // Another owner changed the policy since GET. Review must use this
      // actual preview baseline, not the earlier GET/form snapshot.
      reply.preview.current.tiers[4].min_paid_credits = 450
      return envelope(reply)
    }
    if (path.endsWith('/challenge')) return envelope(challengeReply())
    if (path.endsWith('/confirm')) return envelope({ revision: 13 })
    if (path.endsWith('/resolve')) {
      return envelope({ state: 'pending', revision: null })
    }
    assert.fail('Unexpected path')
  }) as typeof api.post
})
afterEach(() => {
  api.get = originalGet
  api.post = originalPost
  api.put = originalPut
  useAuthStore.getState().auth.reset()
})

async function renderNative() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  })
  queryClient.setQueryData(['system-options'], {
    success: true,
    message: '',
    data: [],
    capabilities: {
      partial: true,
      read_only: false,
      trust_level_benefits: true,
      writable_keys: ['Notice'],
    },
  })
  const route = createRootRoute({
    component: () => (
      <QueryClientProvider client={queryClient}>
        <I18nextProvider i18n={i18n}>
          <SettingsPageProvider actionsContainer={null}>
            <TrustLevelBenefitsSection />
          </SettingsPageProvider>
        </I18nextProvider>
      </QueryClientProvider>
    ),
  })
  const child = createRoute({ getParentRoute: () => route, path: '/' })
  const router = createRouter({
    routeTree: route.addChildren([child]),
    history: createMemoryHistory({ initialEntries: ['/'] }),
  })
  const container = document.createElement('div')
  document.body.appendChild(container)
  const root = createRoot(container)
  await act(async () => {
    root.render(<RouterProvider router={router} />)
    await new Promise((resolve) => setTimeout(resolve, 20))
  })
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 20))
  })
  return {
    container,
    queryClient,
    cleanup: async () => {
      await act(async () => root.unmount())
      queryClient.clear()
      container.remove()
    },
  }
}
function button(container: HTMLElement, label: string) {
  const result = [
    ...container.querySelectorAll<HTMLButtonElement>('button'),
  ].find((item) => item.textContent?.trim() === label)
  assert.ok(result, label)
  return result
}
async function click(element: HTMLElement) {
  await act(async () => {
    element.click()
    await new Promise((resolve) => setTimeout(resolve, 20))
  })
}
async function editThreshold(container: HTMLElement, value: string) {
  const input = container.querySelector<HTMLInputElement>(
    'input[name="tiers.4.min_paid_credits"]'
  )
  assert.ok(input)
  await act(async () => {
    const setter = Object.getOwnPropertyDescriptor(
      domWindow.HTMLInputElement.prototype,
      'value'
    )?.set
    assert.ok(setter)
    setter.call(input, value)
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
}
async function preview(container: HTMLElement) {
  await editThreshold(container, '500')
  await click(button(container, 'Preview trust policy changes'))
}

test('native GET supplies real defaults; null history stays unknown and does not invent benefits', async () => {
  const rendered = await renderNative()
  try {
    assert.equal(
      rendered.container.querySelectorAll('input[name$="min_paid_credits"]')
        .length,
      4
    )
    assert.equal(
      required(
        rendered.container.querySelector<HTMLInputElement>(
          'input[name="tiers.4.min_paid_credits"]'
        )
      ).value,
      '400'
    )
    assert.match(
      rendered.container.textContent ?? '',
      /Historical benefit descriptions are unknown/
    )
    assert.equal(
      rendered.container.querySelectorAll('[role="checkbox"][data-checked]')
        .length,
      0
    )
    assert.equal(writes.length, 0)
    assert.equal(credentialCount, 0)
  } finally {
    await rendered.cleanup()
  }
})

test('no active credential, revoked credential, and RP mismatch are explicitly read only', async () => {
  for (const status of ['missing', 'revoked', 'configuration_mismatch']) {
    read.editors =
      status === 'missing'
        ? []
        : [{ editor_id: 'owner-enrolled-key', version: 1, status }]
    const rendered = await renderNative()
    try {
      assert.equal(rendered.container.querySelector('form'), null)
      assert.match(
        rendered.container.textContent ?? '',
        /No active owner-registered editor credential/
      )
      assert.match(rendered.container.textContent ?? '', /Unknown/)
      assert.equal(writes.length, 0)
    } finally {
      await rendered.cleanup()
    }
  }
})

test('actual before/after values require explicit confirmation before challenge and WebAuthn', async () => {
  const rendered = await renderNative()
  try {
    await preview(rendered.container)
    assert.equal(writes.length, 1)
    assert.equal(credentialCount, 0)
    const row = [...rendered.container.querySelectorAll('tr')].find((item) =>
      item.textContent?.includes('L4 · Cumulative recharge threshold')
    )
    assert.ok(row)
    assert.deepEqual(
      [...row.querySelectorAll('td')].map((item) => item.textContent),
      ['L4 · Cumulative recharge threshold', '450', '500']
    )
    assert.equal(row.getAttribute('data-changed'), 'true')
    assert.equal(
      rendered.container.querySelector('form'),
      null,
      'draft cannot change under an immutable review'
    )
    const confirm = button(
      rendered.container,
      'Confirm changes with security key'
    )
    assert.equal(confirm.disabled, true)
    await click(confirm)
    assert.equal(writes.length, 1)
    await click(
      required(
        rendered.container.querySelector<HTMLElement>('#trust-policy-reviewed')
      )
    )
    assert.equal(confirm.disabled, false)
    assert.ok(!(rendered.container.textContent ?? '').includes(token))
    assert.ok(!(rendered.container.textContent ?? '').includes(digest))
    assert.ok(
      !JSON.stringify(
        rendered.queryClient
          .getQueryCache()
          .getAll()
          .map((q) => q.state.data)
      ).includes(token)
    )
    await click(confirm)
    assert.equal(credentialCount, 1)
    assert.deepEqual(
      writes.map((item) => item.path.split('/').at(-1)),
      ['preview', 'challenge', 'confirm']
    )
    assert.match(
      rendered.container.textContent ?? '',
      /Trust policy confirmed at revision 13/
    )
  } finally {
    await rendered.cleanup()
  }
})

test('lost confirmation remains pending, retains original token, and cannot start another write', async () => {
  const standard = api.post
  api.post = (async (path: string, body: Record<string, unknown>) => {
    if (path.endsWith('/confirm')) {
      writes.push({ path, body })
      throw new Error('lost')
    }
    return standard(path, body)
  }) as typeof api.post
  const rendered = await renderNative()
  try {
    await preview(rendered.container)
    await click(
      required(
        rendered.container.querySelector<HTMLElement>('#trust-policy-reviewed')
      )
    )
    await click(button(rendered.container, 'Confirm changes with security key'))
    assert.match(rendered.container.textContent ?? '', /pending, not confirmed/)
    assert.doesNotMatch(
      rendered.container.textContent ?? '',
      /confirmed at revision/
    )
    assert.equal(
      button(rendered.container, 'Start a new review').disabled,
      true
    )
    assert.deepEqual(required(writes.at(-1)).body, { token })
    const count = writes.length
    await click(button(rendered.container, 'Check review status'))
    assert.equal(writes.length, count + 1)
    assert.equal(
      required(writes.at(-1)).path,
      '/api/option/trust-policy/resolve'
    )
    assert.deepEqual(required(writes.at(-1)).body, { token })
  } finally {
    await rendered.cleanup()
  }
})

test('renewed same-user session retains the review for resolve; switching accounts removes it', async () => {
  const rendered = await renderNative()
  try {
    await preview(rendered.container)
    await click(
      required(
        rendered.container.querySelector<HTMLElement>('#trust-policy-reviewed')
      )
    )
    await act(async () => signIn(7, 'renewed-session'))
    await click(button(rendered.container, 'Confirm changes with security key'))
    assert.equal(writes.length, 1, 'old session cannot request a new challenge')
    await click(button(rendered.container, 'Check review status'))
    assert.deepEqual(required(writes.at(-1)).body, { token })
    await act(async () => signIn(8, 'other-account'))
    assert.equal(
      rendered.container.querySelector(
        '[aria-label="Review trust policy changes"]'
      ),
      null
    )
  } finally {
    await rendered.cleanup()
  }
})

test('native multiplier inputs preserve very small exact ratios without percent rounding', async () => {
  const wire = JSON.stringify(envelope(read).data).replace(
    '"numerator":96,"denominator":100',
    '"numerator":1,"denominator":1000000000000000000'
  )
  api.get = (async () => ({ data: wire })) as typeof api.get
  const rendered = await renderNative()
  try {
    const input = rendered.container.querySelector<HTMLInputElement>(
      'input[name="tiers.4.discount_ratio"]'
    )
    assert.ok(input)
    assert.equal(input.value, '1e-18')
    assert.equal(input.max, '1')
    assert.match(rendered.container.textContent ?? '', /L4 charge multiplier/)
  } finally {
    await rendered.cleanup()
  }
})

function required<T>(value: T | null | undefined): T {
  assert.ok(value !== null && value !== undefined)
  return value
}
