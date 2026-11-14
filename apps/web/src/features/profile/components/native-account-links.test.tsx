/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
// @ts-expect-error Bun's test module is available only in the test runtime.
import { mock } from 'bun:test'
import assert from 'node:assert/strict'
import { after, afterEach, beforeEach, test } from 'node:test'

import axios from 'axios'
import { Window } from 'happy-dom'
import type { ReactNode } from 'react'

import type { FederatedProvider } from '@/features/auth/lib/federated-oauth'
import type { AuthBundle } from '@/stores/auth-store'
import { installSupportedAuthBrowser } from '@/test-support/supported-auth-browser'

import type { UserProfile } from '../types'

const dom = new Window({
  url: 'https://bindings.example.test/profile',
  settings: {
    enableJavaScriptEvaluation: false,
    disableJavaScriptFileLoading: true,
  },
})
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLInputElement',
  'Element',
  'SVGElement',
  'Node',
  'Event',
  'MouseEvent',
  'CustomEvent',
  'MutationObserver',
  'ResizeObserver',
  'requestAnimationFrame',
  'cancelAnimationFrame',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: dom[key],
  })
}
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
mock.module('react-i18next', () => ({
  useTranslation: () => ({ t: (value: string) => value }),
}))
mock.module('sonner', () => ({ toast: { success: () => {}, error: () => {} } }))
mock.module('@/components/dialog', () => ({
  Dialog: ({
    open,
    children,
    footer,
  }: {
    open: boolean
    children: ReactNode
    footer: ReactNode
  }) =>
    open ? (
      <div role='dialog'>
        {children}
        {footer}
      </div>
    ) : null,
}))
mock.module('@/components/confirm-dialog', () => ({
  ConfirmDialog: ({
    open,
    handleConfirm,
  }: {
    open: boolean
    handleConfirm: () => void
  }) =>
    open ? (
      <button type='button' onClick={handleConfirm}>
        Confirm unlink
      </button>
    ) : null,
}))
// Normal supported browser; install after this fixture replaces its globals.
installSupportedAuthBrowser()

const { act, useEffect } = await import('react')
const { createRoot } = await import('react-dom/client')
const { api } = await import('@/lib/api')
const { toast } = await import('sonner')
const {
  AuthCookieBlockedError,
  authCookieStorageMessage,
  getAuthCookieBlockedState,
} = await import('@/lib/auth-cookie')
const { TELEGRAM_BIND_RESULT_MESSAGE } =
  await import('@/features/auth/constants')
const { default: globalI18n } = await import('i18next')
await globalI18n.init({ lng: 'en', resources: { en: { translation: {} } } })
const { useAuthStore } = await import('@/stores/auth-store')
const { NativeAccountLinks } = await import('./native-account-links')
const { WeChatBindDialog } = await import('./dialogs/wechat-bind-dialog')
const { useAccountBindingFlow } =
  await import('../hooks/use-account-binding-flow')
const fixed = {
  google: 'https://accounts.google.com',
  github: 'https://github.com',
  linuxdo: 'https://connect.linux.do',
  discord: 'https://discord.com',
}
const provider: FederatedProvider = {
  id: '73',
  kind: 'wechat',
  slug: 'synthetic-wechat',
  name: 'Synthetic WeChat',
  icon: '',
  revision: '1',
  client_id: '',
  authorization_endpoint: '',
  scopes: '',
  qr_code_url: '',
  native_federated: true,
}
const flow = {
  state: `lmmfe_${'e'.repeat(64)}`,
  expires_at: 2000000000,
  authorization_url: '',
}
function bundle(token = 'synthetic-access'): AuthBundle {
  return {
    access_token: token,
    token_type: 'Bearer',
    access_expires_at: 2000000000,
    user: { id: 501, username: 'synthetic', role: 1 },
    session: {
      sid: 'same-synthetic-sid',
      current: true,
      login_method: 'password',
      ip: '',
      user_agent: '',
      created_at: 1,
      last_active_at: 1,
      expires_at: 2000000000,
    },
  }
}
const errorToasts: string[] = []
const successToasts: string[] = []
beforeEach(() => {
  errorToasts.length = 0
  successToasts.length = 0
  toast.error = (message) => {
    errorToasts.push(String(message))
    return 'error'
  }
  toast.success = (message) => {
    successToasts.push(String(message))
    return 'success'
  }
})
const originalAdapter = api.defaults.adapter
let root: ReturnType<typeof createRoot> | undefined
const profile = {} as UserProfile
async function mount(selected = provider) {
  useAuthStore.getState().auth.setBundle(bundle())
  const host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
  await act(async () => {
    root!.render(
      <NativeAccountLinks
        status={{
          native_federated_auth: true,
          federated_oauth_providers: [selected],
        }}
        onUpdate={() => {}}
      />
    )
  })
  return host
}
function button(host: HTMLElement, label: string) {
  const found = Array.from(host.querySelectorAll('button')).find(
    (b) => b.textContent === label
  )
  assert.ok(found, `${label} missing`)
  return found
}
function adapter(selected = provider) {
  let bound = false
  const calls: string[] = []
  api.defaults.adapter = async (config) => {
    calls.push(config.url!)
    let data: unknown
    if (config.method === 'get') {
      data = {
        contract: 'core.account-links.v1',
        expected_auth_version: bound ? '4' : '3',
        links: bound
          ? [
              {
                issuer: 'urn:lmm:federated:73',
                provider_id: '73',
                slug: selected.slug,
                name: selected.name,
                kind: selected.kind,
                available: true,
                subject: 'synthetic-subject',
                revision: '1',
                active: true,
              },
            ]
          : [],
        providers: [
          ...Object.entries(fixed).map(([provider, issuer]) => ({
            provider,
            issuer,
            can_bind: false,
          })),
          {
            provider: 'federated:73',
            provider_id: '73',
            issuer: 'urn:lmm:federated:73',
            can_bind: true,
            slug: selected.slug,
            name: selected.name,
            kind: selected.kind,
            icon: '',
            qr_code_url: '',
            native_federated: true,
          },
        ],
      }
    } else if (config.url!.endsWith('/begin')) {
      data =
        selected.kind === 'wechat'
          ? flow
          : {
              ...flow,
              authorization_url: `${selected.authorization_endpoint}?client_id=synthetic-client&redirect_uri=${encodeURIComponent(`https://bindings.example.test/oauth/${selected.slug}`)}&response_type=code&scope=openid&state=${flow.state}&nonce=synthetic&code_challenge=${'a'.repeat(43)}&code_challenge_method=S256`,
            }
    } else {
      bound = !config.url!.endsWith('/unlink')
      data = bundle(bound ? 'native-bound-token' : 'native-unlinked-token')
    }
    return {
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: { success: true, data },
    }
  }
  return calls
}
afterEach(async () => {
  await act(async () => {
    root?.unmount()
  })
  root = undefined
  api.defaults.adapter = originalAdapter
  dom.document.body.replaceChildren()
  dom.sessionStorage.clear()
  dom.localStorage.clear()
})
after(() => dom.close())
test('dynamic WeChat binding uses native state/code, same-SID rotation and identity CAS unlink', async () => {
  const calls = adapter()
  const host = await mount()
  assert.match(host.textContent ?? '', /Synthetic WeChat/)
  await act(async () => button(host, 'Bind').click())
  const input = host.querySelector<HTMLInputElement>('#wechat-bind-code')
  assert.ok(input)
  await act(async () => {
    const setValue = Object.getOwnPropertyDescriptor(
      dom.HTMLInputElement.prototype,
      'value'
    )?.set
    assert.ok(setValue)
    setValue.call(input, 'synthetic-code')
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
  await act(async () => {
    host
      .querySelector('form')!
      .dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
  })
  assert.equal(useAuthStore.getState().auth.session?.sid, 'same-synthetic-sid')
  assert.equal(useAuthStore.getState().auth.accessToken, 'native-bound-token')
  assert.match(host.textContent ?? '', /synthetic-subject/)
  await act(async () => button(host, 'Unbind').click())
  await act(async () => button(host, 'Confirm unlink').click())
  assert.equal(
    useAuthStore.getState().auth.accessToken,
    'native-unlinked-token'
  )
  assert.equal(calls.filter((url) => url.endsWith('/finish')).length, 1)
  assert.ok(calls.includes('/api/user/account-links/unlink'))
  assert.equal(
    calls.some((url) => url.includes('/wechat/bind')),
    false
  )
})
test('native popup keeps exact Core URL and requires matching origin, source, configured slug and state', async () => {
  const oidc = {
    ...provider,
    kind: 'oidc' as const,
    slug: 'controlled-sso',
    name: 'Synthetic OIDC',
    client_id: 'synthetic-client',
    authorization_endpoint: 'https://idp.example.test/authorize',
    scopes: 'openid',
  }
  const calls = adapter(oidc)
  const popupStore = new Map<string, string>()
  let navigated = ''
  const messages: unknown[] = []
  const popup = {
    closed: false,
    sessionStorage: {
      getItem: (key: string) => popupStore.get(key) ?? null,
      setItem: (key: string, value: string) => popupStore.set(key, value),
      removeItem: (key: string) => popupStore.delete(key),
    },
    location: {
      replace: (value: string) => {
        navigated = value
      },
    },
    close: () => {},
    postMessage: (value: unknown) => messages.push(value),
  }
  const oldOpen = dom.open
  dom.open = (() => popup) as unknown as typeof dom.open
  try {
    const host = await mount(oidc)
    await act(async () => button(host, 'Bind').click())
    assert.match(navigated, /nonce=synthetic/)
    assert.equal(new URL(navigated).searchParams.get('state'), flow.state)
    const data = {
      type: 'oauth-bind-callback',
      provider: oidc.slug,
      state: flow.state,
      code: 'synthetic-code',
    }
    const { OAUTH_BIND_CALLBACK_MESSAGE } =
      await import('@/features/auth/constants')
    data.type = OAUTH_BIND_CALLBACK_MESSAGE
    for (const [origin, source, patch] of [
      ['https://evil.example.test', popup, {}],
      ['https://bindings.example.test', {}, {}],
      ['https://bindings.example.test', popup, { provider: 'oidc' }],
      [
        'https://bindings.example.test',
        popup,
        { state: `lmmo_${'a'.repeat(64)}` },
      ],
    ] as const) {
      await act(async () =>
        dom.dispatchEvent(
          new dom.MessageEvent('message', {
            origin,
            source: source as unknown as Window,
            data: { ...data, ...patch },
          })
        )
      )
    }
    assert.equal(calls.filter((url) => url.endsWith('/finish')).length, 0)
    await act(async () =>
      dom.dispatchEvent(
        new dom.MessageEvent('message', {
          origin: 'https://bindings.example.test',
          source: popup as unknown as Window,
          data,
        })
      )
    )
    assert.equal(calls.filter((url) => url.endsWith('/finish')).length, 1)
    assert.equal(useAuthStore.getState().auth.accessToken, 'native-bound-token')
    assert.equal(messages.length, 1)
  } finally {
    dom.open = oldOpen
  }
})

let controls: ReturnType<typeof useAccountBindingFlow>
const phaseTag = () => controls.state.phase.tag
async function mountFlow(selected = provider) {
  useAuthStore.getState().auth.setBundle(bundle())
  function Harness() {
    const value = useAccountBindingFlow(
      { native_federated_auth: true, federated_oauth_providers: [selected] },
      () => {}
    )
    useEffect(() => {
      controls = value
    })
    return null
  }
  const host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
  await act(async () => root!.render(<Harness />))
}
function nativeDescriptor() {
  const row = controls.state.view?.providers.find(
    (row) => row.provider === 'federated:73'
  )
  assert.ok(row)
  return row
}
function gate() {
  let release!: () => void
  const promise = new Promise<void>((resolve) => {
    release = resolve
  })
  return { promise, release }
}
function laterBundle() {
  const next = bundle('later-user-access')
  next.user = { ...next.user, id: 502 }
  next.session = { ...next.session, sid: 'later-user-sid' }
  return next
}
test('cancel during native begin releases busy and a late begin cannot reopen the code dialog or retain its ceremony', async () => {
  adapter()
  const base = axios.getAdapter(api.defaults.adapter),
    pending = gate()
  api.defaults.adapter = async (config) => {
    if (config.url!.endsWith('/begin')) await pending.promise
    return base(config)
  }
  await mountFlow()
  let started!: Promise<void>
  await act(async () => {
    started = controls.begin(nativeDescriptor())
  })
  assert.equal(phaseTag(), 'starting')
  assert.equal(controls.busy, true)
  await act(async () => controls.cancel())
  assert.equal(phaseTag(), 'idle')
  assert.equal(controls.busy, false)
  await act(async () => {
    pending.release()
    await started
  })
  assert.equal(controls.codeOperation, null)
  const { readFederatedCeremony } =
    await import('@/features/auth/lib/federated-oauth')
  assert.throws(() =>
    readFederatedCeremony(dom.sessionStorage, flow.state, provider.slug, 'bind')
  )
  await act(async () => controls.begin(nativeDescriptor()))
  assert.equal(phaseTag(), 'awaitingCode')
  await act(async () => controls.cancel())
  assert.equal(controls.busy, false)
})
test('native begin and finish rejection both release busy and require a fresh one-use ceremony', async () => {
  const calls = adapter()
  const base = axios.getAdapter(api.defaults.adapter)
  let rejected = 'begin'
  api.defaults.adapter = async (config) =>
    config.url!.endsWith(`/${rejected}`)
      ? {
          config,
          status: 200,
          statusText: 'OK',
          headers: {},
          data: { success: false },
        }
      : base(config)
  await mountFlow()
  await act(async () => controls.begin(nativeDescriptor()))
  assert.equal(phaseTag(), 'idle')
  assert.equal(controls.busy, false)
  rejected = 'finish'
  await act(async () => controls.begin(nativeDescriptor()))
  await act(async () => {
    await assert.rejects(controls.submitCode('synthetic-code'))
  })
  assert.equal(phaseTag(), 'idle')
  assert.equal(controls.busy, false)
  assert.equal(useAuthStore.getState().auth.accessToken, 'synthetic-access')
  assert.equal(calls.filter((url) => url.endsWith('/finish')).length, 0)
  await act(async () => controls.begin(nativeDescriptor()))
  assert.equal(phaseTag(), 'awaitingCode')
  await act(async () => controls.cancel())
})
test('switching session during begin discards the previous generation without reopening or changing the new account', async () => {
  adapter()
  const base = axios.getAdapter(api.defaults.adapter),
    pending = gate()
  api.defaults.adapter = async (config) => {
    if (config.url!.endsWith('/begin')) await pending.promise
    return base(config)
  }
  await mountFlow()
  let started!: Promise<void>
  await act(async () => {
    started = controls.begin(nativeDescriptor())
  })
  await act(async () => useAuthStore.getState().auth.setBundle(laterBundle()))
  await act(async () => {
    pending.release()
    await started
  })
  assert.equal(phaseTag(), 'idle')
  assert.equal(controls.busy, false)
  assert.equal(controls.codeOperation, null)
  assert.equal(useAuthStore.getState().auth.accessToken, 'later-user-access')
})
function popupFixture() {
  const storage = new Map<string, string>(),
    messages: unknown[] = []
  const popup = {
    closed: false,
    sessionStorage: {
      getItem: (key: string) => storage.get(key) ?? null,
      setItem: (key: string, value: string) => storage.set(key, value),
      removeItem: (key: string) => storage.delete(key),
    },
    location: { replace: () => {} },
    close: () => {
      popup.closed = true
    },
    postMessage: (message: unknown) => messages.push(message),
  }
  const selected = {
    ...provider,
    kind: 'oidc' as const,
    slug: 'controlled-sso',
    name: 'Synthetic OIDC',
    client_id: 'synthetic-client',
    authorization_endpoint: 'https://idp.example.test/authorize',
    scopes: 'openid',
  }
  const previous = dom.open
  dom.open = (() => popup) as unknown as typeof dom.open
  return {
    popup,
    selected,
    messages,
    restore: () => {
      dom.open = previous
    },
    event: (type: string) =>
      new dom.MessageEvent('message', {
        origin: 'https://bindings.example.test',
        source: popup as unknown as Window,
        data: {
          type,
          provider: selected.slug,
          state: flow.state,
          code: 'synthetic-code',
        },
      }),
  }
}
for (const switchSession of [false, true]) {
  test(
    switchSession
      ? 'session change during popup finish cannot rotate the later account or publish a stale success'
      : 'duplicate popup callbacks finish once and release busy after the same-SID rotation',
    async () => {
      const fixture = popupFixture(),
        pending = gate()
      const calls = adapter(fixture.selected),
        base = axios.getAdapter(api.defaults.adapter)
      let finishes = 0
      api.defaults.adapter = async (config) => {
        if (config.url!.endsWith('/finish')) {
          finishes++
          await pending.promise
        }
        return base(config)
      }
      try {
        await mountFlow(fixture.selected)
        await act(async () => controls.begin(nativeDescriptor()))
        const { OAUTH_BIND_CALLBACK_MESSAGE } =
          await import('@/features/auth/constants')
        const event = () => fixture.event(OAUTH_BIND_CALLBACK_MESSAGE)
        await act(async () => {
          dom.dispatchEvent(event())
          dom.dispatchEvent(event())
        })
        assert.equal(finishes, 1)
        assert.equal(phaseTag(), 'finishing')
        assert.equal(controls.busy, true)
        if (switchSession) {
          await act(async () =>
            useAuthStore.getState().auth.setBundle(laterBundle())
          )
        }
        await act(async () => {
          pending.release()
          await new Promise((resolve) => setTimeout(resolve, 0))
        })
        assert.equal(phaseTag(), 'idle')
        assert.equal(controls.busy, false)
        assert.equal(
          useAuthStore.getState().auth.accessToken,
          switchSession ? 'later-user-access' : 'native-bound-token'
        )
        assert.equal(fixture.messages.length, switchSession ? 0 : 1)
        assert.equal(calls.filter((url) => url.endsWith('/finish')).length, 1)
        if (switchSession) assert.equal(fixture.popup.closed, true)
      } finally {
        fixture.restore()
      }
    }
  )
}

test('cancelled popup releases busy, closes only its window and never submits a binding finish', async () => {
  const fixture = popupFixture(),
    calls = adapter(fixture.selected)
  try {
    await mountFlow(fixture.selected)
    await act(async () => controls.begin(nativeDescriptor()))
    assert.equal(phaseTag(), 'awaitingPopup')
    await act(async () => controls.cancel())
    assert.equal(phaseTag(), 'idle')
    assert.equal(controls.busy, false)
    assert.equal(fixture.popup.closed, true)
    assert.equal(calls.filter((url) => url.endsWith('/finish')).length, 0)
  } finally {
    fixture.restore()
  }
})
test('provider-denied popup settles with one failure reply and no empty-code finish', async () => {
  const fixture = popupFixture(),
    calls = adapter(fixture.selected)
  try {
    await mountFlow(fixture.selected)
    await act(async () => controls.begin(nativeDescriptor()))
    const { OAUTH_BIND_CALLBACK_MESSAGE } =
      await import('@/features/auth/constants')
    await act(async () =>
      dom.dispatchEvent(
        new dom.MessageEvent('message', {
          origin: 'https://bindings.example.test',
          source: fixture.popup as unknown as Window,
          data: {
            type: OAUTH_BIND_CALLBACK_MESSAGE,
            provider: fixture.selected.slug,
            state: flow.state,
            error: 'access_denied',
          },
        })
      )
    )
    assert.equal(phaseTag(), 'idle')
    assert.equal(controls.busy, false)
    assert.equal(calls.filter((url) => url.endsWith('/finish')).length, 0)
    assert.deepEqual(
      fixture.messages.map((value) => (value as { success: boolean }).success),
      [false]
    )
  } finally {
    fixture.restore()
  }
})
test('unlink rejection releases busy and keeps the target for an explicit retry', async () => {
  adapter()
  const base = axios.getAdapter(api.defaults.adapter)
  let reject = true
  api.defaults.adapter = async (config) =>
    reject && config.url!.endsWith('/unlink')
      ? {
          config,
          status: 200,
          statusText: 'OK',
          headers: {},
          data: { success: false },
        }
      : base(config)
  await mountFlow()
  await act(async () => controls.begin(nativeDescriptor()))
  await act(async () => controls.submitCode('synthetic-code'))
  const link = controls.state.view?.links.find((link) => link.active)
  assert.ok(link)
  await act(async () => controls.selectTarget(link))
  await act(async () => controls.unlink())
  assert.equal(phaseTag(), 'idle')
  assert.equal(controls.busy, false)
  assert.equal(controls.state.target?.subject, link.subject)
  assert.equal(useAuthStore.getState().auth.accessToken, 'native-bound-token')
  reject = false
  await act(async () => controls.unlink())
  assert.equal(phaseTag(), 'idle')
  assert.equal(controls.state.target, null)
  assert.equal(
    useAuthStore.getState().auth.accessToken,
    'native-unlinked-token'
  )
})

test('equivalent profile object refresh in the actual tab keeps its owned popup and completes exactly once', async () => {
  const fixture = popupFixture()
  const calls = adapter(fixture.selected)
  mock.module('@/hooks/use-status', () => ({
    useStatus: () => ({
      status: {
        native_account_bindings: true,
        native_federated_auth: true,
        federated_oauth_providers: [fixture.selected],
      },
      loading: false,
      capabilitiesReady: true,
    }),
  }))
  const { AccountBindingsTab } = await import('./tabs/account-bindings-tab')
  useAuthStore.getState().auth.setBundle(bundle())
  const host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
  let updates = 0
  const first = { ...profile, id: 501, email: 'synthetic@example.test' }
  const mountedRoot = root
  assert.ok(mountedRoot)
  const render = async (value: UserProfile) => {
    await act(async () =>
      mountedRoot.render(
        <AccountBindingsTab
          profile={value}
          onUpdate={() => {
            updates++
          }}
        />
      )
    )
  }
  try {
    await render(first)
    await act(async () => button(host, 'Bind').click())
    assert.equal(fixture.popup.closed, false)
    const replacement = { ...first }
    assert.notEqual(first, replacement)
    assert.deepEqual(first, replacement)
    await render(replacement)
    assert.equal(fixture.popup.closed, false)
    assert.equal(calls.filter((url) => url.endsWith('/begin')).length, 1)
    assert.equal(
      calls.filter((url) => url === '/api/user/account-links').length,
      1
    )
    const { OAUTH_BIND_CALLBACK_MESSAGE } =
      await import('@/features/auth/constants')
    await act(async () =>
      dom.dispatchEvent(fixture.event(OAUTH_BIND_CALLBACK_MESSAGE))
    )
    assert.equal(calls.filter((url) => url.endsWith('/finish')).length, 1)
    assert.equal(fixture.messages.length, 1)
    assert.equal(useAuthStore.getState().auth.accessToken, 'native-bound-token')
    assert.equal(updates, 1)
  } finally {
    fixture.restore()
  }
})

test('phase snapshots stay immutable through begin, code and finish while same-tick completion is exclusive', async () => {
  const calls = adapter()
  const base = axios.getAdapter(api.defaults.adapter)
  const beginGate = gate(),
    finishGate = gate()
  let finishes = 0
  api.defaults.adapter = async (config) => {
    if (config.url?.endsWith('/begin')) await beginGate.promise
    if (config.url?.endsWith('/finish')) {
      finishes++
      await finishGate.promise
    }
    return base(config)
  }
  await mountFlow()
  let started!: Promise<void>
  await act(async () => {
    started = controls.begin(nativeDescriptor())
  })
  const starting = Object.freeze(controls.state.phase)
  assert.equal(starting.tag, 'starting')
  assert.ok('operationId' in starting)
  assert.deepEqual(Object.keys(starting).sort(), ['operationId', 'tag'])
  await act(async () => {
    beginGate.release()
    await started
  })
  assert.equal(starting.tag, 'starting')
  const code = Object.freeze(controls.state.phase)
  assert.equal(code.tag, 'awaitingCode')
  assert.ok('codeOperation' in code && code.codeOperation)
  Object.freeze(code.codeOperation)
  assert.equal(code.codeOperation.state, flow.state)
  const { OAUTH_BIND_CALLBACK_MESSAGE } =
    await import('@/features/auth/constants')
  await act(async () =>
    dom.dispatchEvent(
      new dom.MessageEvent('message', {
        origin: dom.location.origin,
        source: null,
        data: {
          type: OAUTH_BIND_CALLBACK_MESSAGE,
          provider: provider.slug,
          state: flow.state,
          code: 'unsolicited-popup-code',
        },
      })
    )
  )
  assert.equal(finishes, 0, 'a code dialog does not accept popup messages')
  let finished!: Promise<void>
  await act(async () => {
    finished = controls.submitCode('synthetic-code')
    await assert.rejects(
      controls.submitCode('duplicate-code'),
      axios.CanceledError
    )
    controls.cancel()
  })
  const finishing = Object.freeze(controls.state.phase)
  assert.equal(finishing.tag, 'finishing')
  assert.equal(
    controls.busy,
    true,
    'an in-flight write remains non-cancellable'
  )
  assert.equal(finishes, 1)
  await act(async () => {
    finishGate.release()
    await finished
  })
  assert.equal(phaseTag(), 'idle')
  assert.equal(starting.tag, 'starting')
  assert.equal(code.tag, 'awaitingCode')
  assert.equal(finishing.tag, 'finishing')
  assert.equal(calls.filter((url) => url.endsWith('/finish')).length, 1)
  assert.equal(useAuthStore.getState().auth.accessToken, 'native-bound-token')
})

function watchResources() {
  const intervals = new Set<unknown>()
  const schedule = Object.getOwnPropertyDescriptor(globalThis, 'setInterval')
  const cancel = Object.getOwnPropertyDescriptor(globalThis, 'clearInterval')
  const originalSchedule = globalThis.setInterval
  const originalCancel = globalThis.clearInterval
  Object.defineProperty(globalThis, 'setInterval', {
    configurable: true,
    value: new Proxy(originalSchedule, {
      apply(target, receiver, args) {
        const handle = Reflect.apply(target, receiver, args)
        if (args[1] === 500) intervals.add(handle)
        return handle
      },
    }),
  })
  Object.defineProperty(globalThis, 'clearInterval', {
    configurable: true,
    value: new Proxy(originalCancel, {
      apply(target, receiver, args) {
        intervals.delete(args[0])
        return Reflect.apply(target, receiver, args)
      },
    }),
  })
  return {
    intervals,
    restore: () => {
      for (const handle of intervals) {
        Reflect.apply(originalCancel, globalThis, [handle])
      }
      if (schedule) Object.defineProperty(globalThis, 'setInterval', schedule)
      else Reflect.deleteProperty(globalThis, 'setInterval')
      if (cancel) Object.defineProperty(globalThis, 'clearInterval', cancel)
      else Reflect.deleteProperty(globalThis, 'clearInterval')
    },
  }
}

for (const boundary of [
  'cancel',
  'unmount',
  'same-account-new-session',
] as const) {
  test(`${boundary} releases only owned popup resources and ignores its late callback`, async () => {
    const fixture = popupFixture()
    const resources = watchResources()
    const calls = adapter(fixture.selected)
    let closes = 0
    const close = fixture.popup.close
    fixture.popup.close = () => {
      closes++
      close()
    }
    try {
      await mountFlow(fixture.selected)
      await act(async () => controls.begin(nativeDescriptor()))
      assert.equal(resources.intervals.size, 1)
      if (boundary === 'cancel') await act(async () => controls.cancel())
      if (boundary === 'unmount') {
        await act(async () => root?.unmount())
        root = undefined
      }
      if (boundary === 'same-account-new-session') {
        const next = bundle('renewed-token')
        next.session = { ...next.session, sid: 'renewed-sid' }
        await act(async () => useAuthStore.getState().auth.setBundle(next))
      }
      assert.equal(closes, 1)
      assert.equal(fixture.popup.closed, true)
      assert.equal(resources.intervals.size, 0)
      const { readFederatedCeremony } =
        await import('@/features/auth/lib/federated-oauth')
      assert.throws(() =>
        readFederatedCeremony(
          dom.sessionStorage,
          flow.state,
          fixture.selected.slug,
          'bind'
        )
      )
      const { OAUTH_BIND_CALLBACK_MESSAGE } =
        await import('@/features/auth/constants')
      await act(async () =>
        dom.dispatchEvent(fixture.event(OAUTH_BIND_CALLBACK_MESSAGE))
      )
      assert.equal(calls.filter((url) => url.endsWith('/finish')).length, 0)
      assert.equal(fixture.messages.length, 0)
      assert.equal(
        useAuthStore.getState().auth.accessToken,
        boundary === 'same-account-new-session'
          ? 'renewed-token'
          : 'synthetic-access'
      )
    } finally {
      resources.restore()
      fixture.restore()
    }
  })
}

async function submitWechat(host: HTMLElement) {
  const input = host.querySelector<HTMLInputElement>('#wechat-bind-code')
  assert.ok(input)
  await act(async () => {
    const setValue = Object.getOwnPropertyDescriptor(
      dom.HTMLInputElement.prototype,
      'value'
    )?.set
    assert.ok(setValue)
    setValue.call(input, 'synthetic-code')
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
  const form = host.querySelector('form')
  assert.ok(form)
  await act(async () => {
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
  })
}

for (const phase of ['begin', 'finish', 'unlink'] as const) {
  for (const failure of [
    'cancel',
    'typed-blocked',
    'known-backend',
    'unknown-backend',
  ] as const) {
    test(`${phase} has one notification owner for ${failure} without losing the safe backend code`, async () => {
      const calls = adapter()
      const base = axios.getAdapter(api.defaults.adapter)
      let rejected = false
      let faults = 0
      api.defaults.adapter = async (config) => {
        if (rejected && config.url?.endsWith(`/${phase}`)) {
          faults++
          assert.deepEqual(config.authScope, {
            userId: 501,
            sessionId: 'same-synthetic-sid',
          })
          assert.equal(config.skipBusinessError, true)
          assert.equal(config.skipErrorHandler, true)
          if (failure === 'cancel') {
            throw new axios.CanceledError('synthetic cancellation')
          }
          if (failure === 'typed-blocked') {
            throw Object.assign(
              new AuthCookieBlockedError('storage_unavailable'),
              { config }
            )
          }
          return {
            config,
            status: 200,
            statusText: 'OK',
            headers: {},
            data: {
              success: false,
              code:
                failure === 'known-backend'
                  ? 'MFA_CODE_INVALID'
                  : 'PRIVATE_UNKNOWN_CODE',
              message: 'PRIVATE_BACKEND_DETAIL',
            },
          }
        }
        return base(config)
      }
      if (phase === 'finish') {
        const host = await mount()
        await act(async () => button(host, 'Bind').click())
        rejected = true
        await submitWechat(host)
        assert.equal(host.querySelector('#wechat-bind-code'), null)
      } else {
        await mountFlow()
        if (phase === 'unlink') {
          await act(async () => controls.begin(nativeDescriptor()))
          await act(async () => controls.submitCode('synthetic-code'))
          const target = controls.state.view?.links[0]
          assert.ok(target)
          await act(async () => controls.selectTarget(target))
        }
        rejected = true
        await act(async () =>
          phase === 'begin'
            ? controls.begin(nativeDescriptor())
            : controls.unlink()
        )
        assert.equal(controls.busy, false)
        assert.equal(phaseTag(), 'idle')
      }
      assert.equal(faults, 1)
      assert.equal(
        getAuthCookieBlockedState(),
        null,
        'an isolated typed error does not publish a block'
      )
      assert.deepEqual(
        errorToasts,
        failure === 'cancel'
          ? []
          : [
              failure === 'typed-blocked'
                ? authCookieStorageMessage
                : failure === 'known-backend'
                  ? 'Invalid code'
                  : phase === 'begin'
                    ? 'Account binding failed. Sign in again if your session is older than 10 minutes.'
                    : phase === 'finish'
                      ? 'Request failed'
                      : 'Account binding could not be confirmed. Refresh the list before trying again.',
            ]
      )
      assert.equal(
        errorToasts.some((value) => value.includes('PRIVATE_BACKEND_DETAIL')),
        false
      )
      assert.equal(
        calls.some((url) => url.includes('/wechat/bind')),
        false
      )
    })
  }
}

test('a late code error from a disposed generation stays silent after account A to B to A', async () => {
  adapter()
  const base = axios.getAdapter(api.defaults.adapter)
  const pending = gate()
  let finishes = 0
  api.defaults.adapter = async (config) => {
    if (config.url?.endsWith('/finish')) {
      finishes++
      await pending.promise
      throw Object.assign(new AuthCookieBlockedError('storage_unavailable'), {
        config,
      })
    }
    return base(config)
  }
  const host = await mount()
  await act(async () => button(host, 'Bind').click())
  await submitWechat(host)
  assert.equal(finishes, 1)
  await act(async () => useAuthStore.getState().auth.setBundle(laterBundle()))
  await act(async () => useAuthStore.getState().auth.setBundle(bundle()))
  await act(async () => {
    pending.release()
    await Promise.resolve()
  })
  assert.equal(getAuthCookieBlockedState(), null)
  assert.deepEqual(errorToasts, [])
  assert.deepEqual(successToasts, [])
  assert.equal(useAuthStore.getState().auth.accessToken, 'synthetic-access')
})

function emptyLinks(links: unknown[] = []) {
  return {
    contract: 'core.account-links.v1',
    expected_auth_version: links.length ? '4' : '3',
    links,
    providers: Object.entries(fixed).map(([provider, issuer]) => ({
      provider,
      issuer,
      can_bind: false,
    })),
  }
}

async function mountTelegram(botName?: string, onUpdate = () => {}) {
  useAuthStore.getState().auth.setBundle(bundle())
  const host = document.createElement('div')
  document.body.append(host)
  const localRoot = createRoot(host)
  root = localRoot
  await act(async () =>
    localRoot.render(
      <NativeAccountLinks
        status={{ telegram_oauth: true, telegram_bot_name: botName }}
        onUpdate={onUpdate}
      />
    )
  )
  return host
}

for (const botName of [undefined, '', '   ', ' @ ']) {
  test(`Telegram cannot begin with missing or blank bot name ${JSON.stringify(botName)}`, async () => {
    let requests = 0
    api.defaults.adapter = async (config) => {
      requests++
      assert.equal(config.method, 'get')
      assert.equal(config.url, '/api/user/account-links')
      return {
        config,
        status: 200,
        statusText: 'OK',
        headers: {},
        data: { success: true, data: emptyLinks() },
      }
    }
    const host = await mountTelegram(botName)
    assert.equal(requests, 1)
    assert.equal(
      [...host.querySelectorAll('button')].some(
        (b) => b.textContent === 'Bind' && !b.disabled
      ),
      false
    )
    assert.equal(host.querySelector('script[data-telegram-login]'), null)
  })
}

test('a bound Telegram account still permits unlink when its bot is unavailable', async () => {
  const link = {
    issuer: 'https://telegram.org',
    subject: '123456',
    revision: '1',
    active: true,
  }
  api.defaults.adapter = async (config) => ({
    config,
    status: 200,
    statusText: 'OK',
    headers: {},
    data: { success: true, data: emptyLinks([link]) },
  })
  const host = await mountTelegram('   ')
  assert.match(host.textContent ?? '', /123456/)
  assert.equal(button(host, 'Unbind').disabled, false)
})

test('Telegram completion refreshes actual account links before the profile callback', async () => {
  const trace: string[] = []
  let bound = false
  const token = `lmmt_${'a'.repeat(64)}`
  api.defaults.adapter = async (config) => {
    assert.ok(config.url)
    trace.push(config.url)
    let data: unknown
    if (config.url === '/api/user/account-links') {
      data = emptyLinks(
        bound
          ? [
              {
                issuer: 'https://telegram.org',
                subject: '123456',
                revision: '1',
                active: true,
              },
            ]
          : []
      )
    } else if (config.url === '/api/oauth/telegram/bind/start') {
      assert.equal(config.method, 'post')
      assert.deepEqual(config.authScope, {
        userId: 501,
        sessionId: 'same-synthetic-sid',
      })
      data = {
        flow_token: token,
        callback_url: `https://bindings.example.test/api/oauth/telegram/bind/${token}`,
        expires_at: Math.floor(Date.now() / 1000) + 300,
      }
    } else {
      assert.equal(config.url, '/api/user/self')
      bound = true
      data = { id: 501, telegram_id: '123456' }
    }
    return {
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: { success: true, data },
    }
  }
  const host = await mountTelegram('  synthetic_bot  ', () => {
    trace.push('profile-update')
  })
  await act(async () => button(host, 'Bind').click())
  assert.equal(
    host.querySelector('script')?.getAttribute('data-telegram-login'),
    'synthetic_bot'
  )
  await act(async () =>
    dom.dispatchEvent(
      new dom.MessageEvent('message', {
        origin: dom.location.origin,
        data: {
          type: TELEGRAM_BIND_RESULT_MESSAGE,
          flow_token: token,
          success: true,
        },
      })
    )
  )
  assert.deepEqual(trace, [
    '/api/user/account-links',
    '/api/oauth/telegram/bind/start',
    '/api/user/self',
    '/api/user/account-links',
    'profile-update',
  ])
  assert.match(host.textContent ?? '', /123456/)
  assert.equal(button(host, 'Unbind').disabled, false)
  assert.deepEqual(successToasts, ['Binding successful!'])
  assert.deepEqual(errorToasts, [])
})

test('Telegram links reload failure preserves the error and never claims binding success', async () => {
  let reads = 0,
    updates = 0
  const token = `lmmt_${'a'.repeat(64)}`
  api.defaults.adapter = async (config) => {
    let data: unknown
    if (config.url === '/api/user/account-links') {
      reads++
      if (reads === 2) {
        throw new axios.AxiosError(
          'synthetic links reload failure',
          'ERR_NETWORK',
          config
        )
      }
      data = emptyLinks()
    } else if (config.url === '/api/oauth/telegram/bind/start') {
      data = {
        flow_token: token,
        callback_url: `https://bindings.example.test/api/oauth/telegram/bind/${token}`,
        expires_at: Math.floor(Date.now() / 1000) + 300,
      }
    } else {
      assert.equal(config.url, '/api/user/self')
      data = { id: 501, telegram_id: '123456' }
    }
    return {
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: { success: true, data },
    }
  }
  const host = await mountTelegram('synthetic_bot', () => {
    updates++
  })
  await act(async () => button(host, 'Bind').click())
  await act(async () =>
    dom.dispatchEvent(
      new dom.MessageEvent('message', {
        origin: dom.location.origin,
        data: {
          type: TELEGRAM_BIND_RESULT_MESSAGE,
          flow_token: token,
          success: true,
        },
      })
    )
  )
  assert.equal(reads, 2)
  assert.equal(updates, 0)
  assert.match(host.textContent ?? '', /Failed to load profile/)
  assert.equal(button(host, 'Retry').disabled, false)
  assert.deepEqual(successToasts, [])
  assert.deepEqual(errorToasts, [])
})

test('account A to B to A during Telegram links refresh drops the old list and profile callback', async () => {
  const pending = gate()
  const token = `lmmt_${'a'.repeat(64)}`
  const linkOwners: Array<number | undefined> = []
  let updates = 0
  api.defaults.adapter = async (config) => {
    let data: unknown
    if (config.url === '/api/user/account-links') {
      linkOwners.push(config.authScope?.userId)
      if (linkOwners.length === 2) {
        await pending.promise
        data = emptyLinks([
          {
            issuer: 'https://telegram.org',
            subject: 'OLD_A_SUBJECT',
            revision: '1',
            active: true,
          },
        ])
      } else data = emptyLinks()
    } else if (config.url === '/api/oauth/telegram/bind/start') {
      data = {
        flow_token: token,
        callback_url: `https://bindings.example.test/api/oauth/telegram/bind/${token}`,
        expires_at: Math.floor(Date.now() / 1000) + 300,
      }
    } else {
      assert.equal(config.url, '/api/user/self')
      data = { id: 501, telegram_id: '123456' }
    }
    return {
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: { success: true, data },
    }
  }
  const host = await mountTelegram('synthetic_bot', () => {
    updates++
  })
  await act(async () => button(host, 'Bind').click())
  await act(async () =>
    dom.dispatchEvent(
      new dom.MessageEvent('message', {
        origin: dom.location.origin,
        data: {
          type: TELEGRAM_BIND_RESULT_MESSAGE,
          flow_token: token,
          success: true,
        },
      })
    )
  )
  assert.deepEqual(linkOwners, [501, 501])
  await act(async () => useAuthStore.getState().auth.setBundle(laterBundle()))
  await act(async () => useAuthStore.getState().auth.setBundle(bundle()))
  await act(async () => {
    pending.release()
    await Promise.resolve()
  })
  assert.deepEqual(linkOwners, [501, 501, 502, 501])
  assert.equal(updates, 0)
  assert.doesNotMatch(host.textContent ?? '', /OLD_A_SUBJECT/)
  assert.equal(host.querySelector('script[data-telegram-login]'), null)
  assert.deepEqual(errorToasts, [])
  assert.deepEqual(successToasts, [])
})

for (const failure of ['cancel', 'known-backend', 'unknown-backend'] as const) {
  test(`legacy WeChat ${failure} preserves the safe response and never reports success`, async () => {
    useAuthStore.getState().auth.setBundle(bundle())
    let posts = 0
    let completions = 0
    const changes: boolean[] = []
    api.defaults.adapter = async (config) => {
      posts++
      assert.equal(config.method, 'post')
      assert.equal(config.url, '/api/oauth/wechat/bind')
      assert.equal(config.skipBusinessError, true)
      if (failure === 'cancel') {
        throw new axios.CanceledError('synthetic cancellation')
      }
      return {
        config,
        status: 200,
        statusText: 'OK',
        headers: {},
        data: {
          success: false,
          code:
            failure === 'known-backend'
              ? 'MFA_CODE_INVALID'
              : 'PRIVATE_UNKNOWN_CODE',
          message: 'PRIVATE_BACKEND_DETAIL',
        },
      }
    }
    const host = document.createElement('div')
    document.body.append(host)
    const localRoot = createRoot(host)
    root = localRoot
    await act(async () =>
      localRoot.render(
        <WeChatBindDialog
          open
          qrCodeUrl=''
          onOpenChange={(open) => {
            changes.push(open)
          }}
          onSuccess={() => {
            completions++
          }}
        />
      )
    )
    await submitWechat(host)
    assert.equal(posts, 1)
    assert.equal(completions, 0)
    assert.deepEqual(changes, [])
    assert.deepEqual(successToasts, [])
    assert.deepEqual(
      errorToasts,
      failure === 'cancel'
        ? []
        : [failure === 'known-backend' ? 'Invalid code' : 'Request failed']
    )
  })
}

test('legacy WeChat account A to B to A discards its old response and resets the code draft', async () => {
  useAuthStore.getState().auth.setBundle(bundle())
  const pending = gate()
  let posts = 0
  let completions = 0
  api.defaults.adapter = async (config) => {
    posts++
    assert.equal(config.url, '/api/oauth/wechat/bind')
    await pending.promise
    return {
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: { success: false, code: 'MFA_CODE_INVALID' },
    }
  }
  const host = document.createElement('div')
  document.body.append(host)
  const localRoot = createRoot(host)
  root = localRoot
  await act(async () =>
    localRoot.render(
      <WeChatBindDialog
        open
        qrCodeUrl=''
        onOpenChange={() => {}}
        onSuccess={() => {
          completions++
        }}
      />
    )
  )
  await submitWechat(host)
  assert.equal(posts, 1)
  await act(async () => useAuthStore.getState().auth.setBundle(laterBundle()))
  await act(async () => useAuthStore.getState().auth.setBundle(bundle()))
  await act(async () => {
    pending.release()
    await Promise.resolve()
  })
  assert.equal(
    host.querySelector<HTMLInputElement>('#wechat-bind-code')?.value,
    ''
  )
  assert.equal(completions, 0)
  assert.deepEqual(errorToasts, [])
  assert.deepEqual(successToasts, [])
  assert.equal(
    button(host, 'Bind').disabled,
    true,
    'the next attempt requires a new code'
  )
})

// Terminal Cookie state is deliberately last; never reset it to make fixtures pass.
test('a real indeterminate bind publishes the root block and has no competing toast or page error', async () => {
  adapter()
  const base = axios.getAdapter(api.defaults.adapter)
  let writes = 0
  api.defaults.adapter = async (config) => {
    if (config.url?.endsWith('/begin')) {
      writes++
      throw new axios.AxiosError(
        'synthetic lost receipt',
        'ERR_NETWORK',
        config
      )
    }
    return base(config)
  }
  const host = await mount()
  await act(async () => button(host, 'Bind').click())
  assert.equal(writes, 1)
  assert.equal(getAuthCookieBlockedState(), 'uncertain')
  assert.equal(button(host, 'Bind').disabled, false)
  assert.equal(host.querySelector('[role="alert"]'), null)
  assert.deepEqual(errorToasts, [])
  assert.deepEqual(successToasts, [])
})
