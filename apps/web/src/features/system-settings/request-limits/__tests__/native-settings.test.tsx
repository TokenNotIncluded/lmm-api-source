/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { Window } from 'happy-dom'
import type { ReactNode } from 'react'

const dom = new Window()
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLInputElement',
  'HTMLTextAreaElement',
  'HTMLButtonElement',
  'HTMLFormElement',
  'SVGElement',
  'Node',
  'Element',
  'Event',
  'KeyboardEvent',
  'MouseEvent',
  'PointerEvent',
  'FocusEvent',
  'CustomEvent',
  'MutationObserver',
  'ResizeObserver',
  'getComputedStyle',
  'requestAnimationFrame',
  'cancelAnimationFrame',
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
const { RequestLimitEditor } = await import('../rate-limit-section')
const { AutoGroupEditor } = await import('../auto-groups-section')
const { requestLimitPolicySchema, autoGroupOrderSchema } =
  await import('../native-api')
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })

type Method = (
  url: string,
  body?: unknown,
  config?: unknown
) => Promise<{ data: unknown }>
const client = api as unknown as { get: Method; put: Method }
const original = { get: client.get, put: client.put }
let rendered:
  | {
      host: HTMLDivElement
      root: ReturnType<typeof createRoot>
      cache: InstanceType<typeof QueryClient>
    }
  | undefined
const scope = { userId: 7, sessionId: 'settings-session' }
const initial = {
  version: 1 as const,
  revision: 4,
  policy: {
    enabled: true,
    window_seconds: 60,
    user_limit: 20,
    group_limits: { default: 50 },
  },
}

async function render(element: ReactNode) {
  const host = document.createElement('div')
  document.body.append(host)
  const root = createRoot(host)
  const cache = new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: 0 },
      mutations: { retry: false },
    },
  })
  rendered = { host, root, cache }
  await act(async () => {
    root.render(
      <QueryClientProvider client={cache}>
        <I18nextProvider i18n={i18n}>{element}</I18nextProvider>
      </QueryClientProvider>
    )
  })
  return host
}
async function change(id: string, value: string) {
  const element = document.getElementById(id)
  assert.ok(
    element instanceof HTMLInputElement ||
      element instanceof HTMLTextAreaElement
  )
  const proto =
    element instanceof HTMLInputElement
      ? dom.HTMLInputElement.prototype
      : dom.HTMLTextAreaElement.prototype
  const set = Object.getOwnPropertyDescriptor(proto, 'value')?.set
  assert.ok(set)
  await act(async () => {
    set.call(element, value)
    element.dispatchEvent(new Event('input', { bubbles: true }))
  })
}
async function submit(host: HTMLElement) {
  const form = host.querySelector('form')
  assert.ok(form)
  await act(async () => {
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
    await new Promise((resolve) => setTimeout(resolve, 10))
  })
}
async function clickButton(host: HTMLElement, label: string) {
  const button = Array.from(host.querySelectorAll('button')).find(
    (button) =>
      button.textContent === label ||
      button.getAttribute('aria-label') === label
  )
  assert.ok(button)
  await act(async () => {
    button.click()
    await new Promise((resolve) => setTimeout(resolve, 10))
  })
}
afterEach(async () => {
  if (rendered) {
    await act(async () => rendered?.root.unmount())
    rendered.cache.clear()
    rendered.host.remove()
    rendered = undefined
  }
  client.get = original.get
  client.put = original.put
})
after(() => dom.happyDOM.close())

test('fixed-window policy uses explicit disabled/null semantics and exact group labels', () => {
  assert.equal(
    requestLimitPolicySchema.safeParse({
      enabled: false,
      window_seconds: 60,
      user_limit: null,
      group_limits: {},
    }).success,
    true
  )
  assert.equal(
    requestLimitPolicySchema.safeParse({
      enabled: true,
      window_seconds: 1,
      user_limit: null,
      group_limits: { '国产[Kimi/Deepseek/GLM]': 2 },
    }).success,
    true
  )
  for (const policy of [
    { ...initial.policy, user_limit: 0 },
    { ...initial.policy, window_seconds: 0 },
    { ...initial.policy, window_seconds: 1.5 },
    { ...initial.policy, group_limits: { default: [2, 3] } },
    { ...initial.policy, group_limits: { 'bad group': 5 } },
    { enabled: true, window_seconds: 60, user_limit: null, group_limits: {} },
  ]) {
    assert.equal(requestLimitPolicySchema.safeParse(policy).success, false)
  }
  assert.equal(autoGroupOrderSchema.safeParse([]).success, true)
  assert.equal(
    autoGroupOrderSchema.safeParse(['default', 'default']).success,
    false
  )
  assert.equal(
    autoGroupOrderSchema.safeParse(
      Array.from({ length: 20 }, (_, i) => `group-${i}`)
    ).success,
    true
  )
})

test('rate settings save one native policy with its revision and null user limit', async () => {
  const writes: Array<{ url: string; body: unknown; config: unknown }> = []
  client.put = async (url, body, config) => {
    writes.push({ url, body, config })
    return {
      data: {
        ...initial,
        revision: 5,
        policy: { ...initial.policy, user_limit: null },
      },
    }
  }
  let accepted: unknown
  const host = await render(
    <RequestLimitEditor
      initial={initial}
      scope={scope}
      onSnapshot={(snapshot) => {
        accepted = snapshot
      }}
    />
  )
  await change('native-rate-user', '')
  await submit(host)
  assert.equal(writes.length, 1)
  assert.equal(writes[0]?.url, '/core/v1/admin/request-limits')
  assert.deepEqual(writes[0]?.body, {
    expected_revision: 4,
    policy: { ...initial.policy, user_limit: null },
  })
  assert.equal((accepted as { revision: number }).revision, 5)
  assert.match(host.textContent ?? '', /Setting updated successfully/)
})

test('version conflict preserves rate draft until explicit reload and does not retry writes', async () => {
  let writes = 0
  client.put = async () => {
    writes++
    throw { isAxiosError: true, response: { status: 409 } }
  }
  client.get = async (url) => {
    assert.equal(url, '/core/v1/admin/request-limits')
    return {
      data: {
        ...initial,
        revision: 8,
        policy: {
          ...initial.policy,
          user_limit: 12,
          group_limits: { priority: 80 },
        },
      },
    }
  }
  const host = await render(
    <RequestLimitEditor initial={initial} scope={scope} onSnapshot={() => {}} />
  )
  await change('native-rate-user', '3')
  await change('native-rate-group-name-0', '研发/试用')
  await change('native-rate-group-limit-0', '70')
  await submit(host)
  assert.equal(writes, 1)
  assert.equal(
    host.querySelector<HTMLInputElement>('#native-rate-user')?.value,
    '3'
  )
  assert.match(host.textContent ?? '', /Settings changed\. Reload/)
  assert.equal(
    host.querySelector<HTMLInputElement>('#native-rate-group-name-0')?.value,
    '研发/试用'
  )
  assert.equal(
    host.querySelector<HTMLInputElement>('#native-rate-group-limit-0')?.value,
    '70'
  )
  await clickButton(host, 'Reload current configuration')
  assert.equal(
    host.querySelector<HTMLInputElement>('#native-rate-user')?.value,
    '12'
  )
  assert.equal(
    host.querySelector<HTMLInputElement>('#native-rate-group-name-0')?.value,
    'priority'
  )
  assert.equal(
    host.querySelector<HTMLInputElement>('#native-rate-group-limit-0')?.value,
    '80'
  )
  client.put = async (_url, body) => {
    writes++
    assert.deepEqual(body, {
      expected_revision: 8,
      policy: {
        ...initial.policy,
        user_limit: 13,
        group_limits: { priority: 80 },
      },
    })
    return {
      data: {
        ...initial,
        revision: 9,
        policy: {
          ...initial.policy,
          user_limit: 13,
          group_limits: { priority: 80 },
        },
      },
    }
  }
  await change('native-rate-user', '13')
  await submit(host)
  assert.equal(writes, 2)
})

test('group rows add exact labels, edit counts and remove limits in one versioned save', async () => {
  const writes: unknown[] = []
  client.put = async (_url, body, config) => {
    writes.push(body)
    assert.deepEqual((config as { authScope: unknown }).authScope, scope)
    return {
      data: {
        ...initial,
        revision: 5,
        policy: (body as { policy: unknown }).policy,
      },
    }
  }
  const host = await render(
    <RequestLimitEditor initial={initial} scope={scope} onSnapshot={() => {}} />
  )
  await change('native-rate-group-limit-0', '75')
  await clickButton(host, 'Add group limit')
  assert.equal(document.activeElement?.id, 'native-rate-group-name-1')
  await change('native-rate-group-name-1', '研发/试用')
  await change('native-rate-group-limit-1', '300')
  await clickButton(host, 'Add group limit')
  await change('native-rate-group-name-2', 'unused-but-valid')
  await change('native-rate-group-limit-2', '40')
  await clickButton(host, 'Remove group limit 3')
  assert.equal(writes.length, 0)
  await submit(host)
  assert.deepEqual(writes, [
    {
      expected_revision: 4,
      policy: {
        ...initial.policy,
        group_limits: { default: 75, '研发/试用': 300 },
      },
    },
  ])
  assert.equal(
    Array.from(host.querySelectorAll('button')).find(
      (button) => button.textContent === 'Save rate limits'
    )?.disabled,
    true
  )
})

test('invalid and duplicate group rows stay editable without sending a policy', async () => {
  const writes: unknown[] = []
  client.put = async (_url, body) => {
    writes.push(body)
    return {
      data: {
        ...initial,
        revision: 5,
        policy: (body as { policy: unknown }).policy,
      },
    }
  }
  const host = await render(
    <RequestLimitEditor initial={initial} scope={scope} onSnapshot={() => {}} />
  )
  await clickButton(host, 'Add group limit')
  await change('native-rate-group-name-1', 'default')
  await change('native-rate-group-limit-1', '25')
  await submit(host)
  assert.match(host.textContent ?? '', /Group names must be unique/)
  assert.equal(writes.length, 0)

  for (const [name, limit] of [
    ['', '25'],
    [' default ', '25'],
    ['VIP', ''],
    ['VIP', '0'],
    ['VIP', '1.5'],
    ['VIP', '100000001'],
  ]) {
    await change('native-rate-group-name-1', name)
    await change('native-rate-group-limit-1', limit)
    await submit(host)
    assert.match(host.textContent ?? '', /Enter exact group names/)
    assert.equal(writes.length, 0)
  }
  await change('native-rate-group-name-1', 'Default')
  await change('native-rate-group-limit-1', '25')
  await submit(host)
  assert.deepEqual(writes, [
    {
      expected_revision: 4,
      policy: { ...initial.policy, group_limits: { default: 50, Default: 25 } },
    },
  ])
})

test('removing the last group leaves the user limit and clears the group policy', async () => {
  let saved: unknown
  client.put = async (_url, body) => {
    saved = body
    return {
      data: {
        ...initial,
        revision: 5,
        policy: { ...initial.policy, group_limits: {} },
      },
    }
  }
  const host = await render(
    <RequestLimitEditor initial={initial} scope={scope} onSnapshot={() => {}} />
  )
  await clickButton(host, 'Remove group limit 1')
  assert.match(host.textContent ?? '', /No group limits configured/)
  assert.equal(saved, undefined)
  await submit(host)
  assert.deepEqual(saved, {
    expected_revision: 4,
    policy: { ...initial.policy, group_limits: {} },
  })
})

test('Auto order saves empty inheritance and rejects duplicate lists before writing', async () => {
  const writes: unknown[] = []
  client.put = async (url, body) => {
    assert.equal(url, '/core/v1/admin/routing/auto-groups')
    writes.push(body)
    return { data: { version: 1, revision: 3, groups: [] } }
  }
  const host = await render(
    <AutoGroupEditor
      initial={{ version: 1, revision: 2, groups: ['private', 'default'] }}
      scope={scope}
      onSnapshot={() => {}}
    />
  )
  await change('native-auto-order', '["default","default"]')
  await submit(host)
  assert.equal(writes.length, 0)
  assert.match(host.textContent ?? '', /unique, valid group names/)
  await change('native-auto-order', '[]')
  await submit(host)
  assert.deepEqual(writes, [{ expected_revision: 2, groups: [] }])
  assert.equal(
    host.querySelector<HTMLTextAreaElement>('#native-auto-order')?.value,
    '[]'
  )
})
