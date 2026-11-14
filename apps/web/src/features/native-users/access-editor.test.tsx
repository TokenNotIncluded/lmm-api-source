/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
// @ts-expect-error Bun's test module is only available in the test runtime.
import { afterAll, afterEach, expect, test } from 'bun:test'

import { Window } from 'happy-dom'

import type { UserAccessSnapshot } from './access-contract'

const dom = new Window({ url: 'https://console.example.invalid/users' })
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLButtonElement',
  'HTMLInputElement',
  'HTMLTextAreaElement',
  'HTMLSelectElement',
  'HTMLLabelElement',
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
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: dom[key],
  })
}
Object.defineProperties(globalThis, {
  requestAnimationFrame: {
    configurable: true,
    value: (callback: FrameRequestCallback) => setTimeout(() => callback(0), 0),
  },
  cancelAnimationFrame: {
    configurable: true,
    value: (handle: number) => clearTimeout(handle),
  },
  getComputedStyle: {
    configurable: true,
    value: dom.getComputedStyle.bind(dom),
  },
  IS_REACT_ACT_ENVIRONMENT: { configurable: true, value: true },
})
afterAll(() => dom.close())

const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { readNativeUsersScope } = await import('./access')
const { NativeUserAccess } = await import('./access-editor')
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
const originalAdapter = api.defaults.adapter
const previousAuth = useAuthStore.getState().auth
const userId = '9007199254740993'
const snapshot: UserAccessSnapshot = {
  contract: 'core.admin-user-access.v1',
  user_id: userId,
  auth_version: '9007199254740997',
  active: true,
  account_active: true,
  service_level: 1,
  developer_access_granted: false,
  groups: ['synthetic-private-route'],
}
let root: ReturnType<typeof createRoot> | undefined
let updated = 0
const onUpdated = () => {
  updated++
}
const onAccessDenied = () => {
  throw new Error('Unexpected access denial')
}
const pause = () => new Promise((resolve) => setTimeout(resolve, 20))

function login(sid = 'synthetic-access-editor-session') {
  useAuthStore.getState().auth.setBundle({
    access_token: 'synthetic-access-editor-credential',
    token_type: 'Bearer',
    access_expires_at: 2000000000,
    user: { id: 901, role: 100, username: 'synthetic-admin' },
    session: {
      sid,
      current: true,
      login_method: 'password',
      ip: '127.0.0.1',
      user_agent: 'synthetic',
      created_at: 1,
      last_active_at: 1,
      expires_at: 2000000000,
    },
  })
}

function button(label: string) {
  return Array.from(
    document.querySelectorAll<HTMLButtonElement>('button')
  ).find((element) => element.textContent?.trim() === label)
}

async function click(label: string) {
  const element = button(label)
  if (!element) throw new Error(`Missing button: ${label}`)
  await act(async () => {
    element.click()
    await pause()
  })
}

async function waitFor(condition: () => boolean) {
  const deadline = Date.now() + 5000
  while (Date.now() < deadline) {
    if (condition()) return
    await act(pause)
  }
  expect(condition()).toBe(true)
}

async function edit(level: string, reason: string) {
  const select = document.getElementById(
    `access-level-${userId}`
  ) as HTMLSelectElement | null
  const input = document.getElementById(`access-reason-${userId}`)
  if (!select || !(input instanceof HTMLInputElement)) {
    throw new Error('Missing access inputs')
  }
  await act(async () => {
    select.value = level
    select.dispatchEvent(new Event('change', { bubbles: true }))
    Object.getOwnPropertyDescriptor(
      dom.HTMLInputElement.prototype,
      'value'
    )?.set?.call(input, reason)
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
}

function transport(
  options: {
    read?: () => UserAccessSnapshot | Promise<UserAccessSnapshot>
    write?: (body: Record<string, unknown>) => unknown
  } = {}
) {
  const calls = { reads: 0, writes: [] as Record<string, unknown>[] }
  api.defaults.adapter = async (config) => {
    let data: unknown
    if (config.method === 'get') {
      calls.reads++
      data = await (options.read?.() ?? snapshot)
    } else {
      const body = JSON.parse(String(config.data)) as Record<string, unknown>
      calls.writes.push(body)
      data = options.write?.(body) ?? {
        changed: true,
        user: {
          ...snapshot,
          auth_version: (
            BigInt(String(body.expected_auth_version)) + 1n
          ).toString(),
          service_level: body.service_level,
          developer_access_granted: body.developer_access_granted,
          groups: body.groups,
        },
      }
    }
    return { config, data, status: 200, statusText: 'OK', headers: {} }
  }
  return calls
}

async function mount() {
  login()
  const scope = readNativeUsersScope()
  if (!scope) throw new Error('Missing test scope')
  const host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
  await act(async () => {
    root?.render(
      <I18nextProvider i18n={i18n}>
        <NativeUserAccess
          scope={scope}
          userId={userId}
          onUpdated={onUpdated}
          onAccessDenied={onAccessDenied}
        />
      </I18nextProvider>
    )
    await pause()
  })
  await click('Access settings')
}

afterEach(async () => {
  await act(async () => {
    root?.unmount()
    await pause()
  })
  root = undefined
  document.body.replaceChildren()
  api.defaults.adapter = originalAdapter
  useAuthStore.setState({ auth: previousAuth })
  updated = 0
})

test('loading and the review step never PATCH before explicit confirmation', async () => {
  let release: ((value: UserAccessSnapshot) => void) | undefined
  const calls = transport({
    read: () =>
      new Promise<UserAccessSnapshot>((resolve) => {
        release = resolve
      }),
  })
  await mount()
  expect(document.querySelector('[role="status"]')?.textContent).toBe('Loading')
  expect(button('Review changes')).toBeUndefined()
  expect(calls.writes).toHaveLength(0)
  await act(async () => {
    release?.(snapshot)
    await pause()
  })
  await waitFor(() => Boolean(button('Review changes')))
  expect(button('Review changes')?.disabled).toBe(true)
  await edit('4', 'Synthetic approved account change')
  expect(button('Review changes')?.disabled).toBe(false)
  await click('Review changes')
  expect(
    document.querySelector('[aria-label="Review access changes"]')?.textContent
  ).toContain('Synthetic approved account change')
  expect(document.getElementById(`access-level-${userId}`)).toBeNull()
  expect(calls.writes).toHaveLength(0)
  await click('Confirm changes')
  await waitFor(() => updated === 1)
  expect(calls.writes).toHaveLength(1)
  expect(calls.writes[0].service_level).toBe(4)
})

test('a conflict clears confirmation and requires loading a new snapshot before another save', async () => {
  let current = snapshot
  let conflict = true
  const calls = transport({
    read: () => current,
    write: () => {
      if (conflict) {
        throw Object.assign(new Error('Synthetic conflict'), {
          response: { status: 409 },
        })
      }
      return undefined
    },
  })
  await mount()
  await waitFor(() => Boolean(button('Review changes')))
  await edit('2', 'Synthetic first change')
  await click('Review changes')
  await click('Confirm changes')
  await waitFor(() => Boolean(button('Reload access settings')))
  expect(document.body.textContent).toContain(
    'This user changed. Reload current access settings before saving.'
  )
  expect(
    document.querySelector('[aria-label="Review access changes"]')
  ).toBeNull()
  expect(button('Confirm changes')).toBeUndefined()
  expect(document.querySelector('form')).toBeNull()
  expect(calls.reads).toBe(1)
  expect(calls.writes).toHaveLength(1)
  current = { ...snapshot, auth_version: '9007199254740999', service_level: 3 }
  conflict = false
  await click('Reload access settings')
  await waitFor(() => Boolean(button('Review changes')))
  expect(
    (document.getElementById(`access-level-${userId}`) as HTMLSelectElement)
      .value
  ).toBe('3')
  expect(
    (document.getElementById(`access-reason-${userId}`) as HTMLInputElement)
      .value
  ).toBe('')
  expect(button('Review changes')?.disabled).toBe(true)
  await edit('4', 'Synthetic revised change')
  await click('Review changes')
  expect(calls.writes).toHaveLength(1)
  await click('Confirm changes')
  await waitFor(() => updated === 1)
  expect(calls.reads).toBe(2)
  expect(calls.writes).toHaveLength(2)
  expect(calls.writes[1].expected_auth_version).toBe(current.auth_version)
})

test('a closed account keeps all access inputs and submission disabled', async () => {
  const calls = transport({
    read: () => ({ ...snapshot, account_active: false }),
  })
  await mount()
  await waitFor(() => Boolean(button('Review changes')))
  expect(document.body.textContent).toContain(
    'This account is closed. Access settings cannot be changed.'
  )
  for (const field of ['level', 'developer', 'groups', 'reason']) {
    const element = document.getElementById(`access-${field}-${userId}`)
    expect(
      element?.matches(':disabled, [aria-disabled="true"], [data-disabled]')
    ).toBe(true)
  }
  expect(button('Review changes')?.disabled).toBe(true)
  await act(async () => {
    document
      .querySelector('form')
      ?.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
    await pause()
  })
  expect(calls.writes).toHaveLength(0)
  expect(button('Confirm changes')).toBeUndefined()
})

test('switching the administrator session removes the old scope and its private review', async () => {
  const calls = transport()
  await mount()
  await waitFor(() => Boolean(button('Review changes')))
  await edit('2', 'Synthetic private review')
  await click('Review changes')
  expect(document.body.textContent).toContain('synthetic-private-route')
  expect(button('Confirm changes')).toBeDefined()
  await act(async () => {
    login('synthetic-replacement-session')
    await pause()
  })
  expect(document.querySelector('[role="dialog"]')).toBeNull()
  expect(button('Access settings')).toBeUndefined()
  expect(button('Confirm changes')).toBeUndefined()
  expect(document.body.textContent).not.toContain('synthetic-private-route')
  expect(document.body.textContent).not.toContain('Synthetic private review')
  expect(calls.writes).toHaveLength(0)
})
