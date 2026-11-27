/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
// @ts-expect-error Bun's test module is only available in the test runtime.
import { afterEach, expect, test } from 'bun:test'
import assert from 'node:assert/strict'

import { Window } from 'happy-dom'

import type { NativeUsersQuery } from './contract'

const dom = new Window({ url: 'https://console.example.invalid/users' })
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'customElements',
  'HTMLInputElement',
  'HTMLButtonElement',
  'HTMLSelectElement',
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
const { useAuthStore } = await import('@/stores/auth-store')
const { NativeAdminUsersEntry } = await import('./entry')
const { NativeUsersPage } = await import('./page')
const {
  nativeListFixture,
  nativeUserFixture,
  nativeStatusFixture,
  nativeUsageFixture,
} = await import('./test-fixtures')
const { nativeUserStatisticsFixture } = await import('./detail-test-fixtures')
const originalAdapter = api.defaults.adapter
const previousAuth = useAuthStore.getState().auth
const cleanups: (() => void)[] = []
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
const pause = () => new Promise((resolve) => setTimeout(resolve, 20))
function login(role = 100, sid = 'synthetic-native-page-session') {
  useAuthStore.getState().auth.setBundle({
    access_token: 'synthetic-native-page-credential',
    token_type: 'Bearer',
    access_expires_at: 2000000000,
    user: { id: 901, role, username: 'synthetic-admin' },
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
async function mount(
  search = '',
  options: {
    onQueryChange?: (query: NativeUsersQuery) => void
    captureRender?: (render: (search: string) => Promise<void>) => void
  } = {}
) {
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  cleanups.push(() => {
    root.unmount()
    client.clear()
    container.remove()
  })
  const render = async (nextSearch: string) => {
    await act(async () => {
      root.render(
        <QueryClientProvider client={client}>
          <I18nextProvider i18n={i18n}>
            <NativeAdminUsersEntry>
              {(scope, recheck) => (
                <NativeUsersPage
                  scope={scope}
                  recheck={recheck}
                  search={nextSearch}
                  onQueryChange={options.onQueryChange}
                />
              )}
            </NativeAdminUsersEntry>
          </I18nextProvider>
        </QueryClientProvider>
      )
      await pause()
    })
    await act(pause)
  }
  options.captureRender?.(render)
  await render(search)
  return container
}
function button(container: HTMLElement, label: string) {
  const element = Array.from(
    container.querySelectorAll<HTMLButtonElement>('button')
  ).find((value) => value.textContent === label)
  if (!element) throw new Error(`Missing test button: ${label}`)
  return element
}

async function waitForElement(container: HTMLElement, selector: string) {
  const deadline = Date.now() + 5000
  while (Date.now() < deadline) {
    const element = container.querySelector(selector)
    if (element) return element
    await act(async () => {
      await pause()
    })
  }
  throw new Error(`Expected rendered element: ${selector}`)
}
afterEach(async () => {
  await act(async () => {
    for (const cleanup of cleanups.splice(0)) cleanup()
  })
  api.defaults.adapter = originalAdapter
  useAuthStore.setState({ auth: previousAuth })
})

test('the actual page renders exact identity, memberships and signed units without dollar or legacy defaults', async () => {
  login(10)
  const seen: string[] = []
  api.defaults.adapter = async (config) => {
    seen.push(`${config.method} ${config.url}`)
    return {
      config,
      data:
        config.url === '/api/status'
          ? nativeStatusFixture
          : nativeListFixture(),
      headers: {},
      status: 200,
      statusText: 'OK',
    }
  }
  const container = await mount()
  const row = container.querySelector(
    '[data-native-user-id="9007199254740993"]'
  )
  expect(row).not.toBeNull()
  for (const value of [
    '9007199254740993',
    '9007199254740995',
    '9007199254740997',
    'synthetic-native@example.invalid',
    'alpha',
    'team-beta',
    '-7',
    'Debt magnitude',
    '1970',
    '—',
    'Synthetic research team',
    'Spending enabled',
  ]) {
    expect(row?.textContent).toContain(value)
  }
  expect(row?.textContent).not.toMatch(/\$|USD|L0|Service level/)
  expect(container.querySelector('#native-user-level')).toBeNull()
  expect(row?.querySelector('.native-user-groups')).toBeNull()
  expect(container.textContent).toContain(
    'Open Manage to review account usage, imported counters and available actions.'
  )
  expect(container.textContent).not.toContain('Unavailable user actions')
  expect(container.textContent).not.toContain('Delete user')
  const security = Array.from(
    row?.querySelectorAll<HTMLButtonElement>('button') ?? []
  ).find((button) => button.textContent === 'Security')
  expect(security).toBeDefined()
  expect(security?.disabled).toBe(false)
  expect(seen).toEqual(['get /api/status', 'get /core/v1/admin/users'])
})

test('opening a history-only identity shows its provenance and usage without changing access', async () => {
  login()
  const user = nativeUserFixture({
    identity_origin: 'history_only',
    active: false,
    status: 2,
    account_active: false,
    wallet: { status: 'unavailable', code: 'WALLET_SUBJECT_UNAVAILABLE' },
  })
  const seen: string[] = []
  api.defaults.adapter = async (config) => {
    seen.push(`${config.method} ${config.url}`)
    return {
      config,
      data:
        config.url === '/api/status'
          ? nativeStatusFixture
          : config.url === `/core/v1/admin/users/${user.id}`
            ? {
                version: 1,
                user,
                usage: nativeUsageFixture(),
                statistics: nativeUserStatisticsFixture(user),
              }
            : nativeListFixture([user]),
      headers: {},
      status: 200,
      statusText: 'OK',
    }
  }
  const container = await mount()
  const row = container.querySelector('[data-native-user-id]') as HTMLElement
  expect(row.querySelector('.native-user-row')?.textContent).toContain('+1')
  expect(row.querySelector('.native-user-row')?.textContent).not.toContain(
    'team-beta'
  )
  expect(
    row.querySelector('.native-user-expanded-row')?.hasAttribute('hidden')
  ).toBe(true)
  expect(seen).toEqual(['get /api/status', 'get /core/v1/admin/users'])
  await act(async () => {
    button(row, 'Manage').click()
    await pause()
  })
  await act(async () => {
    await pause()
  })
  expect(
    row.querySelector('.native-user-expanded-row')?.hasAttribute('hidden')
  ).toBe(false)
  const detailHeader = row.querySelector<HTMLElement>(
    '.native-user-expanded-row header'
  )
  const usageSection = row.querySelector<HTMLElement>(
    '[aria-label="Account usage"]'
  )
  assert.ok(detailHeader && usageSection)
  expect(detailHeader.textContent).toContain(user.username)
  expect(detailHeader.textContent).toContain('Security')
  expect(detailHeader.textContent).toContain('Access settings')
  expect(
    detailHeader.compareDocumentPosition(usageSection) &
      Node.DOCUMENT_POSITION_FOLLOWING
  ).not.toBe(0)
  expect(row.textContent).toContain('1,234')
  expect(row.textContent).toContain('9,876,543')
  expect(row.textContent).toContain('250,000 credits')
  expect(row.textContent).toContain('-17')
  expect(row.textContent).toContain('History only')
  expect(row.textContent).toContain('Login identity not migrated')
  expect(row.textContent).not.toContain('Disabled user')
  expect(row.textContent).toContain(
    'Viewing its history does not enable sign-in or spending.'
  )
  expect(row.querySelector('.native-user-wallet')?.textContent).toContain('—')
  expect(row.querySelector('.native-user-wallet')?.textContent).not.toContain(
    '250,000'
  )
  expect(seen).toEqual([
    'get /api/status',
    'get /core/v1/admin/users',
    `get /core/v1/admin/users/${user.id}`,
  ])
})

test('column selection survives a list refresh and team shortcuts use the server filter', async () => {
  login()
  const teams: unknown[] = []
  api.defaults.adapter = async (config) => {
    if (config.url === '/core/v1/admin/users') {
      teams.push(config.params?.team_id)
    }
    return {
      config,
      data:
        config.url === '/api/status'
          ? nativeStatusFixture
          : nativeListFixture(),
      headers: {},
      status: 200,
      statusText: 'OK',
    }
  }
  const container = await mount()
  await act(async () => {
    button(container, 'Columns').click()
    await pause()
  })
  const email = Array.from(
    document.querySelectorAll<HTMLElement>('[role="menuitemcheckbox"]')
  ).find((item) => item.textContent === 'Email')
  expect(email).toBeDefined()
  await act(async () => {
    email?.click()
    await pause()
  })
  expect(container.querySelector('.native-user-email')).toBeNull()
  await act(async () => {
    button(container, 'Refresh users').click()
    await pause()
  })
  await act(async () => {
    await pause()
  })
  expect(container.querySelector('.native-user-email')).toBeNull()
  const team = container.querySelector<HTMLButtonElement>(
    '.native-user-row [aria-label="Filter by team Synthetic research team"]'
  )
  expect(team).not.toBeNull()
  await act(async () => {
    team?.click()
    await pause()
  })
  await act(async () => {
    await pause()
  })
  expect(teams.at(-1)).toBe('12')
  expect(
    (container.querySelector('#native-user-team') as HTMLInputElement).value
  ).toBe('12')
  await act(async () => {
    container
      .querySelector<HTMLButtonElement>(
        '[aria-label="Remove filter: Team #12"]'
      )
      ?.click()
    await pause()
  })
  expect(teams.at(-1)).toBeUndefined()
  expect(
    (container.querySelector('#native-user-team') as HTMLInputElement).value
  ).toBe('')
})

test('false or absent capabilities and an ordinary role produce zero native or legacy user requests', async () => {
  const seen: string[] = []
  api.defaults.adapter = async (config) => {
    seen.push(config.url ?? '')
    return {
      config,
      data: {
        success: true,
        data: {
          admin_users_protocol: 'core.admin-users.v1',
          capabilities: { admin_users_read: false },
        },
      },
      headers: {},
      status: 200,
      statusText: 'OK',
    }
  }
  login()
  const container = await mount()
  expect(container.textContent).toContain('User management is unavailable')
  await act(async () => {
    login(1)
  })
  expect(container.querySelector('[data-native-user-id]')).toBeNull()
  expect(seen).toEqual(['/api/status'])
})

test('unsupported raw saved filters are visible and never fetch an unfiltered native list', async () => {
  login()
  const seen: string[] = []
  api.defaults.adapter = async (config) => {
    seen.push(config.url ?? '')
    return {
      config,
      data: nativeStatusFixture,
      headers: {},
      status: 200,
      statusText: 'OK',
    }
  }
  for (const search of [
    '?l0Only=true',
    '?sortBy=quota',
    '?sortBy=unknown',
    '?risk=high',
    '?pageSize=20',
    '?filter=alice',
    '?role=["10"]',
    '?role=user&role=admin',
    '?keyword=a&keyword=a',
    '?service_level=5',
    '?page_size=101',
    '?sort_by=quota',
  ]) {
    const container = await mount(search)
    expect(container.textContent).toContain(
      'unsupported user filters or sorting'
    )
    expect(container.querySelector('a[href="/users"]')?.textContent).toBe(
      'Clear saved user filters'
    )
    expect(container.querySelector('[data-native-user-id]')).toBeNull()
  }
  expect(seen).toEqual(Array(12).fill('/api/status'))
})

test('known empty profiles and unavailable wallets remain distinct from invented values', async () => {
  login()
  const user = nativeUserFixture({
    profile_available: false,
    username: null,
    display_name: null,
    email: null,
    active: false,
    status: 2,
    account_active: false,
    groups: [],
    wallet: { status: 'unavailable', code: 'WALLET_SUBJECT_UNAVAILABLE' },
    registered_at_unix_ms: null,
  })
  api.defaults.adapter = async (config) => ({
    config,
    data:
      config.url === '/api/status'
        ? nativeStatusFixture
        : nativeListFixture([user]),
    headers: {},
    status: 200,
    statusText: 'OK',
  })
  const container = await mount()
  const row = container.querySelector('[data-native-user-id]')
  expect(row?.textContent).toContain('Profile unavailable')
  expect(row?.textContent).toContain('Sign-in inactive')
  expect(row?.textContent).toContain('Inactive account')
  expect(row?.textContent).not.toContain('History only')
  expect(row?.textContent).not.toContain('Disabled user')
  expect(row?.textContent).toContain('No explicit group memberships')
  expect(row?.textContent).not.toContain('Empty value')
  expect(row?.textContent).not.toContain('Economic position')
  expect(row?.textContent).not.toContain('synthetic-native@example.invalid')
})

test('a disabled user with an active personal account retains its disabled status', async () => {
  login()
  const user = nativeUserFixture({
    active: false,
    status: 2,
    account_active: true,
    wallet: { status: 'unavailable', code: 'WALLET_SUBJECT_UNAVAILABLE' },
  })
  api.defaults.adapter = async (config) => ({
    config,
    data:
      config.url === '/api/status'
        ? nativeStatusFixture
        : nativeListFixture([user]),
    headers: {},
    status: 200,
    statusText: 'OK',
  })
  const container = await mount()
  const status = container.querySelector(
    '[data-native-user-id] .native-user-status'
  )
  expect(status?.textContent).toContain('Disabled user')
  expect(status?.textContent).not.toContain('History only')
  expect(status?.textContent).not.toContain('Inactive account')
})

test('pagination removes previous data immediately, ignores cancelled success and shows failures safely', async () => {
  login()
  let release: (() => void) | undefined
  let secondPageCalls = 0
  const seen: string[] = []
  api.defaults.adapter = async (config) => {
    seen.push(`${config.method} ${config.url}`)
    if (config.url === '/api/status') {
      return {
        config,
        data: nativeStatusFixture,
        headers: {},
        status: 200,
        statusText: 'OK',
      }
    }
    if (config.params.page === 2) {
      secondPageCalls++
      if (secondPageCalls === 1) {
        await new Promise<void>((resolve) => {
          release = resolve
        })
      } else {
        throw Object.assign(new Error('synthetic-private-upstream-query'), {
          response: { status: 503 },
        })
      }
    }
    return {
      config,
      data: nativeListFixture(undefined, config.params.page, 21),
      headers: {},
      status: 200,
      statusText: 'OK',
    }
  }
  const container = await mount()
  expect(container.textContent).toContain('synthetic-native@example.invalid')
  await act(async () => {
    button(container, 'Next user page').click()
    await pause()
  })
  expect(container.querySelector('[data-native-user-id]')).toBeNull()
  await act(async () => {
    button(container, 'Recheck server availability').click()
    await pause()
  })
  await act(async () => {
    release?.()
    await pause()
  })
  expect(container.textContent).toContain('Page 1')
  expect(container.textContent).not.toContain('Page 2')
  await act(async () => {
    button(container, 'Next user page').click()
    await pause()
  })
  expect(container.querySelector('[data-native-user-id]')).toBeNull()
  expect(container.textContent).toContain('No previous results are displayed')
  expect(container.textContent).not.toContain(
    'synthetic-private-upstream-query'
  )
  expect(
    seen.every(
      (value) => value.startsWith('get ') && !value.includes('/api/user/')
    )
  ).toBe(true)
})

test('switching SID while data is pending cannot restore old private results', async () => {
  login()
  let release: (() => void) | undefined
  let reads = 0
  api.defaults.adapter = async (config) => {
    if (config.url === '/api/status') {
      return {
        config,
        data: nativeStatusFixture,
        headers: {},
        status: 200,
        statusText: 'OK',
      }
    }
    if (++reads === 1) {
      await new Promise<void>((resolve) => {
        release = resolve
      })
    } else {
      throw Object.assign(new Error('unavailable'), {
        response: { status: 503 },
      })
    }
    return {
      config,
      data: nativeListFixture(),
      headers: {},
      status: 200,
      statusText: 'OK',
    }
  }
  const container = await mount()
  await act(async () => {
    login(100, 'synthetic-native-page-replacement')
    await pause()
  })
  await act(async () => {
    release?.()
    await pause()
  })
  expect(container.textContent).not.toContain(
    'synthetic-native@example.invalid'
  )
  expect(container.querySelector('[data-native-user-id]')).toBeNull()
  expect(container.textContent).toContain('No previous results are displayed')
})

test('groups page independently, selecting a choice keeps exact membership and every operation remains GET', async () => {
  login()
  const seen: {
    method: string | undefined
    url: string | undefined
    params: Record<string, unknown>
  }[] = []
  api.defaults.adapter = async (config) => {
    seen.push({ method: config.method, url: config.url, params: config.params })
    const data =
      config.url === '/api/status'
        ? nativeStatusFixture
        : config.url?.endsWith('/groups')
          ? {
              version: 1,
              total: 21,
              page: config.params.page,
              page_size: 20,
              items: config.params.page === 1 ? ['alpha'] : [' exact '],
              source: 'enabled_explicit_user_routing_grants',
            }
          : nativeListFixture()
    return { config, data, headers: {}, status: 200, statusText: 'OK' }
  }
  const container = await mount()
  await act(async () => {
    button(container, 'Browse group directory').click()
    await pause()
  })
  expect(container.textContent).toContain('21 groups · Page 1')
  await act(async () => {
    button(container, 'Next group page').click()
    await pause()
  })
  expect(container.textContent).toContain('21 groups · Page 2')
  await act(async () => {
    button(container, ' exact ').click()
  })
  expect(
    container.querySelector<HTMLInputElement>('#native-user-group')?.value
  ).toBe(' exact ')
  await act(async () => {
    const advanced = container.querySelector<HTMLDetailsElement>('form details')
    expect(advanced?.open).toBe(true)
    if (!advanced) throw new Error('Missing advanced user filters')
    button(advanced, 'Apply filters').click()
    await pause()
  })
  expect(seen.at(-1)?.params.group).toBe(' exact ')
  expect(
    seen
      .filter((value) => value.url?.endsWith('/groups'))
      .map((value) => value.params.page)
  ).toEqual([1, 2])
  expect(seen.every((value) => value.method === 'get')).toBe(true)
})

test('malformed successful data never renders a partial or legacy-shaped row', async () => {
  login()
  api.defaults.adapter = async (config) => ({
    config,
    data:
      config.url === '/api/status'
        ? nativeStatusFixture
        : {
            ...nativeListFixture(),
            capabilities: {
              ...nativeListFixture().capabilities,
              implemented_write_actions: ['normal_ban'],
            },
          },
    headers: {},
    status: 200,
    statusText: 'OK',
  })
  const container = await mount()
  expect(container.textContent).toContain('incompatible user data')
  expect(container.textContent).not.toContain(
    'synthetic-native@example.invalid'
  )
  expect(container.querySelector('[data-native-user-id]')).toBeNull()
})

test('saved supported page sizes are represented by a matching select option and exact request', async () => {
  login()
  api.defaults.adapter = async (config) => ({
    config,
    data:
      config.url === '/api/status'
        ? nativeStatusFixture
        : { ...nativeListFixture(), page_size: 37 },
    headers: {},
    status: 200,
    statusText: 'OK',
  })
  const container = await mount('?page_size=37')
  const select = container.querySelector<HTMLSelectElement>(
    '#native-user-page-size'
  )
  expect(select?.value).toBe('37')
  expect(
    Array.from(select?.options ?? []).some((option) => option.value === '37')
  ).toBe(true)
  expect(container.querySelector('[data-native-user-id]')).not.toBeNull()
})

test('native saved URL applies every filter and sort directly to Core without translating values', async () => {
  login()
  const reads: Record<string, unknown>[] = []
  api.defaults.adapter = async (config) => {
    const status = config.url === '/api/status'
    if (!status) {
      expect(config.url).toBe('/core/v1/admin/users')
      reads.push(config.params)
    }
    return {
      config,
      data: status
        ? nativeStatusFixture
        : {
            ...nativeListFixture(),
            total: 100,
            page: config.params.page,
            page_size: config.params.page_size,
          },
      headers: {},
      status: 200,
      statusText: 'OK',
    }
  }
  const params = new URLSearchParams({
    page: '2',
    page_size: '37',
    keyword: JSON.stringify('9007199254740993'),
    group: ' exact ',
    role: 'admin',
    status: 'disabled',
    team_id: '12',
    sort_by: 'display_name',
    sort_order: 'asc',
  })
  const container = await mount(`?${params}`)
  expect(container.querySelector('[data-native-user-id]')).not.toBeNull()
  expect(reads).toEqual([
    {
      page: 2,
      page_size: 37,
      keyword: '9007199254740993',
      group: ' exact ',
      role: 'admin',
      status: 'disabled',
      team_id: '12',
      sort_by: 'display_name',
      sort_order: 'asc',
    },
  ])
  await act(async () => {
    button(container, 'Apply filters').click()
    await pause()
  })
  expect(reads.at(-1)).toEqual({ ...reads[0], page: 1 })
})

test('a denied group read clears all private projections and requires explicit retry before any new read', async () => {
  login()
  const seen: string[] = []
  api.defaults.adapter = async (config) => {
    seen.push(config.url ?? '')
    if (config.url?.endsWith('/groups')) {
      throw Object.assign(new Error('synthetic local denial'), {
        response: { status: 401 },
      })
    }
    return {
      config,
      data:
        config.url === '/api/status'
          ? nativeStatusFixture
          : nativeListFixture(),
      headers: {},
      status: 200,
      statusText: 'OK',
    }
  }
  const container = await mount()
  expect(container.textContent).toContain('synthetic-native@example.invalid')
  await act(async () => {
    button(container, 'Browse group directory').click()
    await pause()
  })
  expect(container.textContent).toContain(
    'Administrator access or multi-factor verification is required.'
  )
  expect(container.textContent).not.toContain(
    'synthetic-native@example.invalid'
  )
  expect(container.querySelector('[data-native-user-id]')).toBeNull()
  const deniedCalls = [...seen]
  await act(async () => {
    await pause()
  })
  expect(seen).toEqual(deniedCalls)
  await act(async () => {
    button(container, 'Try again').click()
    await pause()
  })
  await act(async () => {
    await pause()
  })
  expect(seen.slice(deniedCalls.length)).toEqual([
    '/api/status',
    '/core/v1/admin/users',
  ])
  expect(container.textContent).toContain('synthetic-native@example.invalid')
  expect(container.textContent).not.toContain('Group directory is unavailable.')
})

test('a denied list replacement unmounts groups and aborts an outstanding group page response', async () => {
  login()
  let userReads = 0
  let release: (() => void) | undefined
  let pendingSignal: AbortSignal | undefined
  api.defaults.adapter = async (config) => {
    if (config.url === '/api/status') {
      return {
        config,
        data: nativeStatusFixture,
        headers: {},
        status: 200,
        statusText: 'OK',
      }
    }
    if (config.url?.endsWith('/groups')) {
      if (config.params.page === 2) {
        pendingSignal = config.signal as AbortSignal
        await new Promise<void>((resolve) => {
          release = resolve
        })
      }
      return {
        config,
        data: {
          version: 1,
          items: ['synthetic-private-group'],
          total: 21,
          page: config.params.page,
          page_size: 20,
          source: 'enabled_explicit_user_routing_grants',
        },
        headers: {},
        status: 200,
        statusText: 'OK',
      }
    }
    if (++userReads > 1) {
      throw Object.assign(new Error('synthetic local denial'), {
        response: { status: 403 },
      })
    }
    return {
      config,
      data: nativeListFixture(),
      headers: {},
      status: 200,
      statusText: 'OK',
    }
  }
  const container = await mount()
  await act(async () => {
    button(container, 'Browse group directory').click()
    await pause()
  })
  expect(container.textContent).toContain('synthetic-private-group')
  await act(async () => {
    button(container, 'Next group page').click()
    await pause()
  })
  expect(pendingSignal?.aborted).toBe(false)
  await act(async () => {
    button(container, 'Apply filters').click()
    await pause()
  })
  expect(container.textContent).toContain(
    'Administrator access or multi-factor verification is required.'
  )
  expect(container.textContent).not.toContain('synthetic-private-group')
  expect(container.textContent).not.toContain(
    'synthetic-native@example.invalid'
  )
  expect(pendingSignal?.aborted).toBe(true)
  await act(async () => {
    release?.()
    await pause()
  })
  expect(container.textContent).not.toContain('synthetic-private-group')
  expect(container.querySelector('[data-native-user-id]')).toBeNull()
  expect(userReads).toBe(2)
})

test('clear filters restores the default query and collapses user details initially', async () => {
  login()
  const reads: Record<string, unknown>[] = []
  api.defaults.adapter = async (config) => {
    if (config.url !== '/api/status') reads.push(config.params)
    return {
      config,
      data:
        config.url === '/api/status'
          ? nativeStatusFixture
          : nativeListFixture(),
      headers: {},
      status: 200,
      statusText: 'OK',
    }
  }
  const container = await mount('?keyword=alice&status=disabled')
  expect(
    container
      .querySelector('[data-native-user-id] button[aria-controls]')
      ?.getAttribute('aria-expanded')
  ).toBe('false')
  expect(
    container.querySelector<HTMLDetailsElement>('form details')?.open
  ).toBe(false)
  await act(async () => {
    button(container, 'Clear filters').click()
    await pause()
  })
  expect(reads.at(-1)?.keyword).toBe('')
  expect(reads.at(-1)?.status).toBe('')
  expect(reads.at(-1)?.page).toBe(1)
  expect(
    container.querySelector<HTMLInputElement>('#native-user-keyword')?.value
  ).toBe('')
})

test('page selection exports only selected users and is cleared before replacement data loads', async () => {
  login()
  const first = nativeUserFixture()
  const second = nativeUserFixture({
    id: '9007199254740994',
    username: 'synthetic-second-user',
    wallet: { status: 'unavailable', code: 'WALLET_PROJECTION_UNAVAILABLE' },
  })
  const reads: number[] = []
  api.defaults.adapter = async (config) => {
    if (config.url !== '/api/status') reads.push(config.params.page)
    return {
      config,
      data:
        config.url === '/api/status'
          ? nativeStatusFixture
          : nativeListFixture(
              config.params.page === 1 ? [first, second] : [second],
              config.params.page,
              21
            ),
      headers: {},
      status: 200,
      statusText: 'OK',
    }
  }
  let download: Blob | undefined
  const originalCreate = URL.createObjectURL
  const originalRevoke = URL.revokeObjectURL
  URL.createObjectURL = (blob) => {
    download = blob as Blob
    return 'blob:synthetic-users-export'
  }
  URL.revokeObjectURL = () => {}
  const blockNavigation = (event: Event) => {
    if ((event.target as Element)?.closest('a[download]')) {
      event.preventDefault()
    }
  }
  document.addEventListener('click', blockNavigation)
  cleanups.push(() => {
    URL.createObjectURL = originalCreate
    URL.revokeObjectURL = originalRevoke
    document.removeEventListener('click', blockNavigation)
  })
  const container = await mount()
  expect(container.textContent).toContain('Showing 1–2 of 21 users')
  await act(async () => {
    container
      .querySelector<HTMLElement>(
        `[data-native-user-id="${first.id}"] [role="checkbox"]`
      )
      ?.click()
  })
  expect(container.textContent).toContain('Selected: 1')
  await act(async () => {
    button(container, 'Export selected').click()
  })
  const csv = await download?.text()
  expect(csv).toContain("'9007199254740993")
  expect(csv).toContain('synthetic-native-user')
  expect(csv).not.toContain('synthetic-second-user')
  expect(reads).toEqual([1])
  await act(async () => {
    button(container, 'Next user page').click()
    await pause()
  })
  expect(container.textContent).not.toContain('Selected:')
  expect(button(container, 'Export current page').disabled).toBe(false)
  expect(container.textContent).toContain('Showing 21–21 of 21 users')
  expect(reads).toEqual([1, 2])
})

test('copy uses applied filters and sorting a table heading sends an exact server query', async () => {
  login()
  const reads: Record<string, unknown>[] = []
  api.defaults.adapter = async (config) => {
    if (config.url !== '/api/status') reads.push(config.params)
    return {
      config,
      data:
        config.url === '/api/status'
          ? nativeStatusFixture
          : nativeListFixture(),
      headers: {},
      status: 200,
      statusText: 'OK',
    }
  }
  let copied = ''
  const originalWrite = navigator.clipboard.writeText
  navigator.clipboard.writeText = async (value) => {
    copied = value
  }
  cleanups.push(() => {
    navigator.clipboard.writeText = originalWrite
  })
  const container = await mount('?keyword=%229007199254740993%22&group=+exact+')
  await act(async () => {
    button(container, 'Copy filter link').click()
    await pause()
  })
  const { nativeQueryFromSearch } = await import('./search')
  expect(nativeQueryFromSearch(new URL(copied).search)).toMatchObject({
    keyword: '9007199254740993',
    group: ' exact ',
  })
  expect(container.textContent).toContain('Filter link copied.')
  await act(async () => {
    button(container, 'Username').click()
    await pause()
  })
  const ascendingHeader = await waitForElement(
    container,
    'th[aria-sort="ascending"]'
  )
  expect(reads.at(-1)).toMatchObject({
    keyword: '9007199254740993',
    group: ' exact ',
    sort_by: 'username',
    sort_order: 'asc',
    page: 1,
  })
  expect(ascendingHeader.textContent).toBe('Username')
  await act(async () => {
    button(container, 'Username').click()
    await pause()
  })
  await waitForElement(container, 'th[aria-sort="descending"]')
  expect(reads.at(-1)?.sort_order).toBe('desc')
  const details = container.querySelector<HTMLButtonElement>(
    '[data-native-user-id] button[aria-controls]'
  )
  await act(async () => {
    details?.click()
  })
  expect(details?.getAttribute('aria-expanded')).toBe('true')
  expect(
    container.querySelector(`#${details?.getAttribute('aria-controls')}`)
      ?.textContent
  ).toContain('Security')
})

test('create user opens a focused dialog and submits one account without inline form clutter', async () => {
  login()
  const writes: Record<string, unknown>[] = []
  const keywords: string[] = []
  api.defaults.adapter = async (config) => {
    if (config.method === 'post') {
      writes.push(JSON.parse(String(config.data)))
      return {
        config,
        data: { version: 1, user_id: '902', platform_role: 'user' },
        headers: {},
        status: 201,
        statusText: 'Created',
      }
    }
    if (config.url !== '/api/status') keywords.push(config.params.keyword)
    return {
      config,
      data:
        config.url === '/api/status'
          ? nativeStatusFixture
          : nativeListFixture([
              nativeUserFixture({
                id: config.params.keyword === '902' ? '902' : '901',
              }),
            ]),
      headers: {},
      status: 200,
      statusText: 'OK',
    }
  }
  const container = await mount()
  expect(document.getElementById('native-create-password')).toBeNull()
  await act(async () => {
    button(container, 'Create user').click()
    await pause()
  })
  const dialog = (await waitForElement(
    document.body,
    '[role="dialog"]'
  )) as HTMLElement
  expect(dialog.textContent).toContain('Create a personal account.')
  expect(button(dialog, 'Create user').disabled).toBe(true)
  await act(async () => {
    for (const [id, value] of [
      ['native-create-username', 'synthetic-new-user'],
      ['native-create-display-name', 'New user'],
      ['native-create-password', 'test-only-password'],
    ]) {
      const input = document.getElementById(id) as HTMLInputElement
      Object.getOwnPropertyDescriptor(
        dom.HTMLInputElement.prototype,
        'value'
      )?.set?.call(input, value)
      input.dispatchEvent(new Event('input', { bubbles: true }))
    }
  })
  expect(button(dialog, 'Create user').disabled).toBe(false)
  await act(async () => {
    button(dialog, 'Create user').click()
    await pause()
  })
  await act(pause)
  expect(writes).toEqual([
    {
      username: 'synthetic-new-user',
      display_name: 'New user',
      password: 'test-only-password',
    },
  ])
  expect(document.getElementById('native-create-password')).toBeNull()
  expect(keywords.at(-1)).toBe('902')
  expect(container.querySelector('#native-user-details-902')).not.toBeNull()
})

test('status shortcuts apply the current search and keep team, role and sort filters', async () => {
  login()
  const reads: Record<string, unknown>[] = []
  api.defaults.adapter = async (config) => {
    if (config.url !== '/api/status') reads.push(config.params)
    return {
      config,
      data:
        config.url === '/api/status'
          ? nativeStatusFixture
          : nativeListFixture(
              [
                nativeUserFixture({
                  active: config.params.status !== 'disabled',
                  status: config.params.status === 'disabled' ? 2 : 1,
                }),
              ],
              config.params.page,
              60
            ),
      headers: {},
      status: 200,
      statusText: 'OK',
    }
  }
  const container = await mount(
    '?keyword=alice&team_id=12&role=user&sort_by=username&sort_order=asc&page=2'
  )
  const keyword = container.querySelector<HTMLInputElement>(
    '#native-user-keyword'
  )
  assert.ok(keyword)
  await act(async () => {
    Object.getOwnPropertyDescriptor(
      dom.HTMLInputElement.prototype,
      'value'
    )?.set?.call(keyword, 'updated search')
    keyword.dispatchEvent(new Event('input', { bubbles: true }))
  })
  const status = container.querySelector<HTMLElement>(
    '[role=group][aria-label="User status"]'
  )
  assert.ok(status)
  await act(async () => {
    button(status, 'Sign-in inactive').click()
    await pause()
  })
  expect(reads.at(-1)).toMatchObject({
    keyword: 'updated search',
    team_id: '12',
    role: 'user',
    sort_by: 'username',
    sort_order: 'asc',
    status: 'disabled',
    page: 1,
  })
  expect(button(status, 'Sign-in inactive').getAttribute('aria-pressed')).toBe(
    'true'
  )
  await act(async () => {
    button(status, 'All users').click()
    await pause()
  })
  expect(reads.at(-1)).toMatchObject({
    keyword: 'updated search',
    team_id: '12',
    role: 'user',
    status: '',
    page: 1,
  })
  expect(button(status, 'All users').getAttribute('aria-pressed')).toBe('true')
})

test('page jump validates the requested range, preserves filters and clears page selection', async () => {
  login()
  const reads: Record<string, unknown>[] = []
  api.defaults.adapter = async (config) => {
    if (config.url !== '/api/status') reads.push(config.params)
    return {
      config,
      data:
        config.url === '/api/status'
          ? nativeStatusFixture
          : nativeListFixture(undefined, config.params.page, 205),
      headers: {},
      status: 200,
      statusText: 'OK',
    }
  }
  const container = await mount('?keyword=alice&group=alpha')
  await act(async () => {
    container
      .querySelector<HTMLElement>('[data-native-user-id] [role=checkbox]')
      ?.click()
  })
  expect(container.textContent).toContain('Selected: 1')
  const page = container.querySelector<HTMLInputElement>(
    '#native-user-page-number'
  )
  assert.ok(page)
  const changePage = async (value: string) => {
    await act(async () => {
      Object.getOwnPropertyDescriptor(
        dom.HTMLInputElement.prototype,
        'value'
      )?.set?.call(page, value)
      page.dispatchEvent(new Event('input', { bubbles: true }))
    })
  }
  expect(page.max).toBe('11')
  for (const invalid of ['', '0', '12', '1.5']) {
    await changePage(invalid)
    expect(button(container, 'Go').disabled).toBe(true)
    await act(async () => {
      page.form?.dispatchEvent(
        new Event('submit', { bubbles: true, cancelable: true })
      )
    })
    expect(reads).toHaveLength(1)
  }
  await changePage('7')
  expect(button(container, 'Go').disabled).toBe(false)
  await act(async () => {
    button(container, 'Go').click()
    await pause()
  })
  expect(reads.at(-1)).toMatchObject({
    keyword: 'alice',
    group: 'alpha',
    page: 7,
    page_size: 20,
  })
  expect(container.textContent).toContain('Page 7 of 11')
  expect(container.textContent).not.toContain('Selected:')
  expect(page.value).toBe('7')
  expect(button(container, 'Go').disabled).toBe(true)
})

test('user filters notify navigation and restore browser history without losing column choices', async () => {
  login()
  const reads: Record<string, unknown>[] = []
  const navigated: NativeUsersQuery[] = []
  const { nativeUsersSearch } = await import('./search')
  const user = nativeUserFixture({ username: '9007199254740993' })
  api.defaults.adapter = async (config) => {
    if (config.url !== '/api/status') reads.push(config.params)
    return {
      config,
      data:
        config.url === '/api/status'
          ? nativeStatusFixture
          : nativeListFixture([user], config.params.page, 60),
      headers: {},
      status: 200,
      statusText: 'OK',
    }
  }
  let render!: (search: string) => Promise<void>
  const original = '?keyword=alice&team_id=%229007199254740993%22'
  const container = await mount(original, {
    onQueryChange: (query) => navigated.push(query),
    captureRender: (next) => {
      render = next
    },
  })
  await act(async () => {
    button(container, 'Columns').click()
    await pause()
  })
  const email = Array.from(
    document.querySelectorAll<HTMLElement>('[role="menuitemcheckbox"]')
  ).find((item) => item.textContent === 'Email')
  assert.ok(email)
  await act(async () => {
    email.click()
    await pause()
  })
  await act(async () => {
    button(container, 'Next user page').click()
    await pause()
  })
  expect(navigated).toHaveLength(1)
  expect(navigated[0]).toMatchObject({
    keyword: 'alice',
    team_id: '9007199254740993',
    page: 2,
  })
  const beforeNavigation = reads.length
  await render(nativeUsersSearch(navigated[0]))
  expect(reads).toHaveLength(beforeNavigation)
  expect(container.querySelector('.native-user-email')).toBeNull()
  await render(original)
  expect(reads.at(-1)).toMatchObject({
    keyword: 'alice',
    team_id: '9007199254740993',
    page: 1,
  })
  expect(container.querySelector('.native-user-email')).toBeNull()
  expect(container.textContent).toContain('Page 1 of 3')
  expect(navigated).toHaveLength(1)
  const link = container.querySelector<HTMLAnchorElement>(
    '.native-user-expanded-row a[href^="/usage-logs/common"]'
  )
  assert.ok(link)
  const { defaultParseSearch } = await import('@tanstack/react-router')
  expect(defaultParseSearch(new URL(link.href).search)).toEqual({
    username: '9007199254740993',
    source: 'native',
    scope: 'all',
  })
})
