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
const { claimGift } = await import('../api')
const { toast } = await import('sonner')
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
  const host = await render(<GiftCard capabilitiesReady onRefresh={() => {}} />)
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

test('unverified or unavailable compensation support reads only live capabilities on retry', async () => {
  let refreshed = 0
  let reads = 0
  api.get = (async () => {
    reads++
    throw new Error('unexpected gift discovery')
  }) as typeof api.get
  useAuthStore.setState({
    auth: {
      ...previousAuth,
      user: { id: 7 },
      session: { sid: 'gift-test' },
    } as never,
  })
  for (const ready of [false, true]) {
    const host = await render(
      <GiftCard
        capabilitiesReady={ready}
        enabled={!ready}
        onRefresh={() => {
          refreshed++
        }}
      />
    )
    assert.match(
      host.textContent ?? '',
      ready
        ? /does not support compensation gifts/
        : /capabilities have not been loaded/
    )
    await act(async () => host.querySelector('button')?.click())
  }
  assert.equal(reads, 0)
  assert.equal(refreshed, 2)
})

test('an unadvertised gift endpoint distinguishes the old native fallback, transient 503 and a real list', async () => {
  useAuthStore.setState({
    auth: {
      ...previousAuth,
      user: { id: 7 },
      session: { sid: 'gift-test' },
    } as never,
  })
  let calls = 0
  api.get = (async () => {
    calls++
    if (calls <= 2)
      throw Object.assign(new Error('Request failed with status code 503'), {
        isAxiosError: true,
        response: {
          status: 503,
          data: {
            error: {
              code:
                calls === 1
                  ? 'core_migration_incomplete'
                  : 'service_temporarily_unavailable',
            },
          },
        },
      })
    return { data: { success: true, data: [] } }
  }) as typeof api.get
  const host = await render(<GiftCard capabilitiesReady onRefresh={() => {}} />)
  const settle = () => new Promise((resolve) => setTimeout(resolve, 30))
  await act(async () => {
    await settle()
  })
  assert.match(host.textContent ?? '', /does not support compensation gifts/)
  await act(async () => {
    host.querySelector('button')?.click()
    await settle()
  })
  assert.match(host.textContent ?? '', /Unable to load compensation gifts/)
  assert.match(host.textContent ?? '', /temporarily unavailable/)
  assert.doesNotMatch(host.textContent ?? '', /Request failed with status/)
  await act(async () => {
    host.querySelector('button')?.click()
    await settle()
  })
  assert.equal(host.textContent, '')
  assert.equal(calls, 3)
})

test('compensation claims require the initiating account, gift, positive receipt and explicit duplicate status', async () => {
  const original = api.post
  const input = { giftId: 31, userId: 7, sessionId: 'gift-test' }
  const claim = {
    id: 71,
    gift_id: 31,
    user_id: 7,
    quota: 10,
    created_at: 1700000000,
  }
  let payload: unknown
  api.post = (async (url, body, config) => {
    assert.equal(url, '/api/user/gift/31/claim')
    assert.equal(body, undefined)
    assert.deepEqual(config?.authScope, { userId: 7, sessionId: 'gift-test' })
    return { data: payload }
  }) as typeof api.post
  try {
    for (const already_claimed of [false, true]) {
      payload = { success: true, data: { claim, already_claimed } }
      assert.deepEqual(await claimGift(input), payload)
    }
    for (const data of [
      undefined,
      { claim },
      { claim, already_claimed: 'false' },
      ...[{ gift_id: 99 }, { user_id: 63 }, { id: 0 }, { quota: 0 }].map(
        (change) => ({ claim: { ...claim, ...change }, already_claimed: false })
      ),
    ]) {
      payload = { success: true, data }
      await assert.rejects(claimGift(input))
    }
    payload = { success: false, message: 'Gift expired' }
    assert.deepEqual(await claimGift(input), payload)
  } finally {
    api.post = original
  }
})

test('a listed gift posts once on click and an ambiguous response refreshes its actual claimed state', async () => {
  useAuthStore.setState({
    auth: {
      ...previousAuth,
      user: { id: 7, role: 1 },
      session: { sid: 'gift-test' },
    } as never,
  })
  const previous = {
    post: api.post,
    success: toast.success,
    error: toast.error,
  }
  const now = Math.floor(Date.now() / 1000)
  let writes = 0,
    reads = 0,
    successes = 0,
    unknown = 0
  api.get = (async (url) => {
    assert.equal(url, '/api/user/gift')
    reads++
    return {
      data: {
        success: true,
        data: [
          {
            id: 31,
            title: 'Maintenance compensation',
            description: '',
            quota: 10,
            start_at: now - 300,
            end_at: now + 300,
            min_used_quota: 0,
            min_account_age_days: 0,
            enabled: true,
            claimed: writes > 0,
            eligible: writes === 0,
            created_at: now - 600,
          },
        ],
      },
    }
  }) as typeof api.get
  api.post = (async () => {
    writes++
    return { data: { success: true } }
  }) as typeof api.post
  toast.success = (() => {
    successes++
    return 1
  }) as typeof toast.success
  toast.error = ((message) => {
    assert.match(String(message), /could not be confirmed/)
    unknown++
    return 2
  }) as typeof toast.error
  const settle = () => new Promise((resolve) => setTimeout(resolve, 35))
  try {
    const host = await render(
      <GiftCard capabilitiesReady onRefresh={() => {}} />
    )
    await act(async () => {
      await settle()
    })
    assert.match(host.textContent ?? '', /Maintenance compensation/)
    assert.equal(reads, 1)
    assert.equal(writes, 0)
    const claim = [...host.querySelectorAll('button')].find(
      (b) => b.textContent === 'Claim'
    )
    assert.ok(claim)
    assert.equal(claim.disabled, false)
    await act(async () => {
      claim.click()
      await settle()
    })
    assert.equal(writes, 1)
    assert.equal(reads, 2)
    assert.equal(successes, 0)
    assert.equal(unknown, 1)
    assert.match(host.textContent ?? '', /Claimed/)
    assert.equal(host.querySelector('button')?.disabled, true)
  } finally {
    api.post = previous.post
    toast.success = previous.success
    toast.error = previous.error
  }
})
