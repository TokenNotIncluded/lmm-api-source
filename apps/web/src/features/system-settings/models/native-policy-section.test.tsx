/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, afterEach, test } from 'node:test'

import { Window } from 'happy-dom'

const dom = new Window()
for (const [name, value] of Object.entries({
  window: dom,
  document: dom.document,
  navigator: dom.navigator,
  HTMLElement: dom.HTMLElement,
  Element: dom.Element,
  Node: dom.Node,
  Event: dom.Event,
  MouseEvent: dom.MouseEvent,
  MutationObserver: dom.MutationObserver,
  ResizeObserver: dom.ResizeObserver,
  HTMLInputElement: dom.HTMLInputElement,
  HTMLButtonElement: dom.HTMLButtonElement,
  DocumentFragment: dom.DocumentFragment,
  ShadowRoot: dom.ShadowRoot,
  requestAnimationFrame: dom.requestAnimationFrame.bind(dom),
  cancelAnimationFrame: dom.cancelAnimationFrame.bind(dom),
  getComputedStyle: dom.getComputedStyle.bind(dom),
})) {
  Object.defineProperty(globalThis, name, { configurable: true, value })
}
Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
  configurable: true,
  value: true,
})

const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider, notifyManager } =
  await import('@tanstack/react-query')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { SettingsPageProvider } =
  await import('../components/settings-page-context')
const { RoutingReliabilitySection } =
  await import('./routing-reliability-section')
const { GlobalSettingsCard } = await import('./global-settings-card')
const { ChannelAffinitySection } = await import('../general/channel-affinity')
const { NativeChannelOverridesDialog } =
  await import('@/features/channels/components/dialogs/native-channel-overrides-dialog')
notifyManager.setScheduler(queueMicrotask)
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
const previous = useAuthStore.getState().auth
const adapter = api.defaults.adapter
let dispose: (() => Promise<void>) | undefined
afterEach(async () => {
  await dispose?.()
  dispose = undefined
  api.defaults.adapter = adapter
  useAuthStore.setState({ auth: previous })
  document.body.replaceChildren()
})
after(() => dom.close())

async function until(check: () => boolean) {
  for (let attempt = 0; attempt < 100 && !check(); attempt++) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 10))
    })
  }
  assert.ok(check(), 'Routing policy did not reach the expected state')
}

function login() {
  useAuthStore.getState().auth.setBundle({
    access_token: 'synthetic-pricing-session',
    token_type: 'Bearer',
    access_expires_at: 2_000_000_000,
    user: { id: 501, role: 100, username: 'synthetic-admin' },
    session: {
      sid: 'synthetic-pricing-sid',
      current: true,
      login_method: 'password',
      ip: '127.0.0.1',
      user_agent: 'synthetic',
      created_at: 1,
      last_active_at: 1,
      expires_at: 2_000_000_000,
    },
  })
}

const policies = [
  ['routing_reliability', RoutingReliabilitySection, false],
  ['routing_reliability', RoutingReliabilitySection, true],
  ['routing_reliability', RoutingReliabilitySection, false, true],
  ['channel_affinity', ChannelAffinitySection, false],
  ['global_model_policy', GlobalSettingsCard, false],
] as const
for (const [section, Section, conflict, mismatchedReadback] of policies) {
  test(
    section +
      (mismatchedReadback
        ? ' detects a mismatched active policy on readback'
        : conflict
          ? ' holds stale publication until explicit readback'
          : ' previews, confirms, and reads back without overwriting other routing fields'),
    async () => {
      login()
      const path = '/core/v1/admin/routing/configuration'
      const routes = [{ model: 'unchanged', future_field: 9 }]
      const prices = { version: 4, rules: [{ rates: { input: 1 } }] }
      const bindings = { future_binding: { id: 52 } }
      let config = {
        version: { routing: 23, pricing: 4 },
        policy_sections: [section],
        routing: {
          version: 23,
          routes,
          other_policy: { enabled: true },
        } as Record<string, unknown>,
        prices,
        bindings,
      }
      const requests: string[] = []
      const digest = 'a'.repeat(64)
      api.defaults.adapter = async (req) => {
        requests.push(`${req.method} ${req.url}`)
        let data: unknown
        if (
          req.url === '/core/v1/admin/routing/channel-affinity/cache' &&
          req.method === 'get'
        ) {
          data = {
            enabled: false,
            total: 0,
            unknown: 0,
            by_rule_name: {},
            cache_capacity: 100000,
            cache_algo: 'last_success',
          }
        } else if (req.method === 'get' && req.url === path) data = config
        else {
          const body = JSON.parse(req.data)
          assert.deepEqual(body.expected_version, { routing: 23, pricing: 4 })
          assert.deepEqual(body.prices, prices)
          assert.deepEqual(body.bindings, bindings)
          assert.deepEqual(body.routing.routes, routes)
          assert.deepEqual(body.routing.other_policy, { enabled: true })
          assert.equal(body.routing.version, 24)
          const policy = body.routing[section]
          if (section === 'routing_reliability') {
            assert.equal(policy.probe_interval_seconds, 30)
            assert.equal(policy.disable_threshold_seconds, 5)
            assert.equal(policy.disable_status_codes, '401')
          } else if (section === 'channel_affinity') {
            assert.deepEqual(
              policy.rules.map(
                (r: { key_sources: unknown[] }) => r.key_sources
              ),
              [
                [{ type: 'gjson', path: 'prompt_cache_key' }],
                [{ type: 'gjson', path: 'metadata.user_id' }],
              ]
            )
            assert.ok(
              policy.rules.every(
                (r: { skip_retry_on_failure: boolean; ttl_seconds: number }) =>
                  r.skip_retry_on_failure && r.ttl_seconds === 0
              )
            )
          } else {
            assert.equal(policy.honor_request_service_tier, true)
            assert.deepEqual(policy.thinking_model_blacklist, [
              'moonshotai/kimi-k2-thinking',
              'kimi-k2-thinking',
            ])
          }
          if (req.method === 'post' && req.url === `${path}/preview`) {
            data = {
              saved: false,
              expected_version: body.expected_version,
              routing: body.routing,
              confirmation_digest: digest,
            }
          } else {
            assert.equal(req.method, 'put')
            assert.equal(req.url, path)
            assert.equal(body.confirmation_digest, digest)
            if (conflict) throw new Error('stale routing version')
            config = {
              ...config,
              version: { routing: 24, pricing: 4 },
              routing: body.routing,
            }
            if (mismatchedReadback) {
              config.routing[section] = {
                ...policy,
                probe_interval_seconds: 45,
              }
            }
            data = { version: config.version }
          }
        }
        return {
          config: req,
          headers: {},
          status: 200,
          statusText: 'OK',
          data: { success: true, data },
        }
      }
      const client = new QueryClient({
        defaultOptions: { queries: { retry: false } },
      })
      const container = document.createElement('div')
      const actions = document.createElement('div')
      document.body.append(container, actions)
      const root = createRoot(container)
      dispose = async () => {
        await act(async () => root.unmount())
        client.clear()
      }
      await act(async () =>
        root.render(
          <QueryClientProvider client={client}>
            <I18nextProvider i18n={i18n}>
              <SettingsPageProvider actionsContainer={actions}>
                <Section />
              </SettingsPageProvider>
            </I18nextProvider>
          </QueryClientProvider>
        )
      )
      const button = (label: string) =>
        [...document.querySelectorAll<HTMLButtonElement>('button')].find(
          (b) => b.textContent === label
        )
      await until(() => !!button('Preview changes'))
      if (section === 'global_model_policy') {
        const label = [...container.querySelectorAll('label')].find(
          (element) => element.textContent === 'Use the requested service tier'
        )
        assert.ok(label)
        const toggle = container.querySelector<HTMLElement>(
          `[role="switch"][aria-labelledby="${label.id}"]`
        )
        assert.ok(toggle)
        assert.equal(toggle.getAttribute('aria-checked'), 'false')
        await act(async () => toggle.click())
      }
      await act(async () => button('Preview changes')?.click())
      await until(() => !!button('Confirm'))
      assert.ok(!requests.includes(`put ${path}`))
      assert.ok(!document.body.textContent?.includes('future_binding'))
      await act(async () => button('Confirm')?.click())
      if (mismatchedReadback) {
        await until(() => !!button('Reload configuration'))
        assert.equal(button('Preview changes')?.disabled, true)
        assert.equal(requests.filter((v) => v === `put ${path}`).length, 1)
        return
      }
      if (conflict) {
        await until(() => !!button('Reload configuration'))
        assert.equal(button('Preview changes')?.disabled, true)
        assert.equal(requests.filter((v) => v === `put ${path}`).length, 1)
        await act(async () => button('Reload configuration')?.click())
        await until(() => !button('Preview changes')?.disabled)
        assert.equal(requests.filter((v) => v === `put ${path}`).length, 1)
        return
      }
      await until(() => !!container.textContent?.includes('Current version 24'))
      assert.equal(requests.filter((v) => v === `put ${path}`).length, 1)
      assert.equal(requests.filter((v) => v === `get ${path}`).length, 2)
    }
  )
}

test('channel override validation protects request fields before dispatch and accepts clearing the final published map', async () => {
  login()
  const config = {
    version: { routing: 23, pricing: 2 },
    policy_sections: ['channel_overrides'],
    routing: {
      version: 23,
      channel_overrides: {
        '52': { parameters: { temperature: 1 }, headers: {}, status_codes: {} },
      },
    } as Record<string, unknown>,
    prices: { version: 2, rules: [] },
    bindings: {},
  }
  const methods: string[] = []
  const digest = 'b'.repeat(64)
  api.defaults.adapter = async (req) => {
    let data: unknown = config
    if (req.method !== 'get') {
      methods.push(String(req.method))
      const body = JSON.parse(req.data)
      assert.deepEqual(body.routing.channel_overrides, {})
      assert.deepEqual(body.expected_version, { routing: 23, pricing: 2 })
      const canonical = { version: 24 }
      if (req.method === 'post') {
        data = {
          saved: false,
          expected_version: config.version,
          routing: canonical,
          confirmation_digest: digest,
        }
      } else {
        assert.equal(req.method, 'put')
        assert.equal(body.confirmation_digest, digest)
        config.routing = canonical
        config.version = { routing: 24, pricing: 2 }
        data = { version: config.version }
      }
    }
    return {
      config: req,
      headers: {},
      status: 200,
      statusText: 'OK',
      data: { success: true, data },
    }
  }
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const root = createRoot(
    document.body.appendChild(document.createElement('div'))
  )
  let published = 0
  dispose = async () => {
    await act(async () => root.unmount())
    client.clear()
  }
  await act(async () =>
    root.render(
      <QueryClientProvider client={client}>
        <I18nextProvider i18n={i18n}>
          <NativeChannelOverridesDialog
            id={52}
            onClose={() => {}}
            onPublished={() => {
              published++
            }}
          />
        </I18nextProvider>
      </QueryClientProvider>
    )
  )
  async function click(label: string) {
    const element = [
      ...document.querySelectorAll<HTMLButtonElement>('button'),
    ].find((item) => item.textContent === label)
    assert.ok(element, label)
    await act(async () => element.click())
  }
  async function editField(label: string, text: string) {
    const field = document.querySelector<HTMLTextAreaElement>(
      `textarea[id$="-${label}"]`
    )
    const setter = Object.getOwnPropertyDescriptor(
      dom.HTMLTextAreaElement.prototype,
      'value'
    )?.set
    assert.ok(field && setter)
    await act(async () => {
      setter.call(field, text)
      field.dispatchEvent(
        new dom.Event('input', { bubbles: true }) as unknown as Event
      )
    })
  }
  await until(() => !!document.querySelector('textarea[id$="-Parameters"]'))
  for (const [field, invalid] of [
    ['Parameters', '{"model":"other"}'],
    ['Parameters', '{"stream":false}'],
    ['Parameters', '{"top_p":0}'],
    ['Header Override', '{"authorization":"synthetic-test-value"}'],
    ['Header Override', '{"x-client-version":"invalid\\nline"}'],
    ['Status Code Mapping', '{"401":200}'],
    ['Status Code Mapping', '{"200":502}'],
  ]) {
    await editField(field, invalid)
    await click('Preview changes')
    assert.ok(
      document.body.textContent?.includes(
        'Invalid JSON format or values out of allowed range'
      )
    )
    assert.deepEqual(methods, [])
    await editField(field, '{}')
  }
  await click('Preview changes')
  await until(
    () => !!document.body.textContent?.includes('Configuration preview')
  )
  assert.deepEqual(methods, ['post'])
  await click('Confirm')
  await until(() => published === 1)
  assert.deepEqual(methods, ['post', 'put'])
})
