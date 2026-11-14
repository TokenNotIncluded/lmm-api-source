/*
Copyright (C) 2026 LIghtJUNction

SPDX-License-Identifier: AGPL-3.0-or-later
*/
// @ts-expect-error Bun's test module is available only in the test runtime.
import { mock } from 'bun:test'
import assert from 'node:assert/strict'
import { after, afterEach, describe, test } from 'node:test'

import { Window } from 'happy-dom'

import { installSupportedAuthBrowser } from '@/test-support/supported-auth-browser'

import {
  EXTERNAL_BROWSER_PRESETS,
  type ExternalBrowserProvider,
} from '../lib/external-oauth'
import type { SystemStatus } from '../types'

const dom = new Window({ url: 'https://site.example.test/sign-in' })
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'Node',
  'Element',
  'Event',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: dom[key],
  })
}
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
const calls: { path: string; payload: unknown; options: unknown }[] = []
const navigations: string[] = []
const errors: string[] = []
let logoutCalls = 0
let response: unknown
let pendingResponse: Promise<unknown> | undefined
Object.defineProperty(dom.location, 'assign', {
  configurable: true,
  value: (url: string) => navigations.push(url),
})
const t = (key: string) => key
mock.module('react-i18next', () => ({ useTranslation: () => ({ t }) }))
mock.module('sonner', () => ({
  toast: { error: (value: string) => errors.push(value), success: () => {} },
}))
mock.module('@/lib/api', () => ({
  api: {
    post: async (path: string, payload: unknown, options: unknown) => {
      calls.push({ path, payload, options })
      return { data: pendingResponse ? await pendingResponse : response }
    },
  },
  isAuthBundle: () => false,
}))
mock.module('../api', () => ({
  logout: async () => {
    logoutCalls++
    return { success: true }
  },
  startTelegramLogin: async () => '',
  telegramLogin: async () => ({ success: false }),
}))
mock.module('./use-auth-redirect', () => ({
  useAuthRedirect: () => ({ handleLoginSuccess: async () => {} }),
}))
// Normal supported browser; install after this fixture replaces its globals.
installSupportedAuthBrowser()

const { act, useLayoutEffect } = await import('react')
const { createRoot } = await import('react-dom/client')
const { useOAuthLogin } = await import('./use-oauth-login')
let root: ReturnType<typeof createRoot> | undefined
let login: () => Promise<void>
let controls: ReturnType<typeof useOAuthLogin>
function Harness({
  status,
  provider,
  legal,
}: {
  status: SystemStatus
  provider: ExternalBrowserProvider
  legal: boolean
}) {
  const flow = useOAuthLogin(status, '/dashboard', legal)
  useLayoutEffect(() => {
    controls = flow
    switch (provider) {
      case 'github':
        login = flow.handleGitHubLogin
        break
      case 'discord':
        login = flow.handleDiscordLogin
        break
      case 'linuxdo':
        login = flow.handleLinuxDOLogin
        break
    }
  }, [flow, provider])
  return null
}
const providers: ExternalBrowserProvider[] = ['github', 'discord', 'linuxdo']
function configured(provider: ExternalBrowserProvider): SystemStatus {
  return {
    external_oauth_browser: true,
    [`${provider}_oauth`]: true,
    [`${provider}_client_id`]: 'synthetic-client',
  }
}
function validResponse(provider: ExternalBrowserProvider) {
  const preset = EXTERNAL_BROWSER_PRESETS[provider]
  const url = new URL(preset.authorization_endpoint)
  for (const [key, value] of Object.entries({
    client_id: 'synthetic-client',
    response_type: 'code',
    scope: preset.scope,
    state: 'server-generated-state',
    redirect_uri: `https://site.example.test${preset.callbacks[0]}`,
    provider_option: 'preserved-value',
  })) {
    url.searchParams.set(key, value)
  }
  if (preset.pkce) {
    url.searchParams.set('code_challenge', 'server-generated-proof')
    url.searchParams.set('code_challenge_method', 'S256')
  }
  return { success: true, data: { authorization_url: url.toString() } }
}
async function mount(
  provider: ExternalBrowserProvider,
  status = configured(provider),
  legal = false
) {
  const host = document.createElement('div')
  document.body.append(host)
  const currentRoot = createRoot(host)
  root = currentRoot
  await act(async () =>
    currentRoot.render(
      <Harness status={status} provider={provider} legal={legal} />
    )
  )
}
afterEach(async () => {
  if (root) await act(async () => root?.unmount())
  root = undefined
  document.body.replaceChildren()
  calls.length = navigations.length = errors.length = 0
  logoutCalls = 0
  pendingResponse = undefined
})
after(() => {
  mock.restore()
  dom.close()
})

describe('built-in external browser hook contract', () => {
  for (const provider of providers) {
    test(`${provider} starts the browser-owned flow and preserves its URL without legacy session reset`, async () => {
      response = validResponse(provider)
      await mount(provider, configured(provider), true)
      await act(async () => login())
      assert.deepEqual(calls, [
        {
          path: `/api/oauth/${provider}/start`,
          payload: { accepted_legal: true },
          options: { skipAuthRefresh: true },
        },
      ])
      assert.deepEqual(navigations, [
        validResponse(provider).data.authorization_url,
      ])
      assert.equal(logoutCalls, 0)
      assert.deepEqual(errors, [])
    })

    test(`${provider} preserves false consent and refuses required legal consent before the start`, async () => {
      response = validResponse(provider)
      await mount(provider, {
        ...configured(provider),
        privacy_policy_enabled: true,
      })
      await act(async () => login())
      assert.deepEqual(calls, [])
      assert.deepEqual(navigations, [])
      assert.equal(logoutCalls, 0)
      assert.equal(errors.length, 1)
    })

    test(`${provider} never invents legal consent when the site does not require it`, async () => {
      response = validResponse(provider)
      await mount(provider)
      await act(async () => login())
      assert.deepEqual(
        calls.map((call) => call.payload),
        [{ accepted_legal: false }]
      )
      assert.equal(navigations.length, 1)
    })

    test(`${provider} does not fall back when browser capability is not advertised`, async () => {
      await mount(
        provider,
        { ...configured(provider), external_oauth_browser: false },
        true
      )
      await act(async () => login())
      assert.deepEqual(calls, [])
      assert.equal(logoutCalls, 0)
      assert.deepEqual(navigations, [])
    })

    test(`${provider} rejects a malformed native-token or foreign response`, async () => {
      await mount(provider)
      for (const malformedResponse of [
        { success: true, data: { access_token: 'not-a-browser-flow' } },
        {
          success: true,
          data: { authorization_url: 'https://foreign.example.test' },
        },
      ]) {
        response = malformedResponse
        await act(async () => login())
      }
      assert.equal(calls.length, 2)
      assert.equal(errors.length, 2)
      assert.deepEqual(navigations, [])
      assert.equal(logoutCalls, 0)
    })

    test(`${provider} synchronously ignores double starts while its cookie-binding request is pending`, async () => {
      let resolve!: (value: unknown) => void
      pendingResponse = new Promise((done) => {
        resolve = done
      })
      await mount(provider)
      let first!: Promise<void>
      await act(async () => {
        first = login()
        await login()
      })
      assert.equal(calls.length, 1)
      assert.deepEqual(navigations, [])
      await act(async () => {
        resolve(validResponse(provider))
        await first
      })
      assert.equal(calls.length, 1)
      assert.equal(navigations.length, 1)
    })
  }
})

test('all browser strategies share pending ownership and unmount cancels a late redirect', async () => {
  let release!: (value: unknown) => void
  pendingResponse = new Promise((resolve) => {
    release = resolve
  })
  await mount('github')
  let first!: Promise<void>
  await act(async () => {
    first = controls.handleGitHubLogin()
    await controls.handleLinuxDOLogin()
  })
  assert.equal(calls.length, 1)
  const activeRoot = root
  assert.ok(activeRoot)
  await act(async () => activeRoot.unmount())
  root = undefined
  await act(async () => {
    release(validResponse('github'))
    await first
  })
  assert.deepEqual(navigations, [])
  assert.deepEqual(errors, [])
})

test('a browser authorization response cannot redirect after a newer account replaces its captured session', async () => {
  const { useAuthStore } = await import('@/stores/auth-store')
  const original = useAuthStore.getState().auth
  let release!: (value: unknown) => void
  pendingResponse = new Promise((resolve) => {
    release = resolve
  })
  await mount('linuxdo')
  let first!: Promise<void>
  await act(async () => {
    first = login()
  })
  try {
    useAuthStore.getState().auth.setBundle({
      access_token: 'synthetic-later-token',
      token_type: 'Bearer',
      access_expires_at: 2000000000,
      user: { id: 901, username: 'synthetic-later', role: 1 },
      session: {
        sid: 'synthetic-later-sid',
        current: true,
        login_method: 'password',
        ip: '',
        user_agent: '',
        created_at: 1,
        last_active_at: 1,
        expires_at: 2000000000,
      },
    })
    await act(async () => {
      release(validResponse('linuxdo'))
      await first
    })
    assert.equal(calls.length, 1)
    assert.deepEqual(navigations, [])
    assert.deepEqual(errors, [])
  } finally {
    useAuthStore.setState({ auth: original })
  }
})

test('GitHub strategy retains its 20-second timeout policy and rejects a late authorization redirect', async () => {
  const original = globalThis.setTimeout
  const timers: Array<ReturnType<typeof setTimeout>> = []
  let fire: (() => void) | undefined
  globalThis.setTimeout = ((
    callback: (...args: unknown[]) => void,
    delay?: number,
    ...args: unknown[]
  ) => {
    if (delay === 20000) fire = () => callback(...args)
    const timer = original(callback, delay, ...args)
    if (delay === 20000) timers.push(timer)
    return timer
  }) as typeof setTimeout
  let release!: (value: unknown) => void
  pendingResponse = new Promise((resolve) => {
    release = resolve
  })
  try {
    await mount('github')
    let first!: Promise<void>
    await act(async () => {
      first = login()
    })
    assert.ok(fire)
    await act(async () => fire!())
    assert.equal(
      controls.githubButtonText,
      'Request timed out, please refresh and restart GitHub login'
    )
    assert.equal(controls.githubButtonDisabled, true)
    assert.equal(controls.isLoading, false)
    await act(async () => {
      release(validResponse('github'))
      await first
      await login()
    })
    assert.equal(calls.length, 1)
    assert.deepEqual(navigations, [])
  } finally {
    globalThis.setTimeout = original
    for (const timer of timers) clearTimeout(timer)
  }
})
