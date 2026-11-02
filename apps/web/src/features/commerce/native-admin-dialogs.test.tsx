/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, test } from 'node:test'

import { Window } from 'happy-dom'

const dom = new Window({ url: 'https://synthetic.example/subscriptions' })
for (const name of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLInputElement',
  'HTMLButtonElement',
  'SVGElement',
  'Element',
  'Node',
  'Event',
  'MouseEvent',
  'CustomEvent',
  'MutationObserver',
  'ResizeObserver',
  'DocumentFragment',
  'ShadowRoot',
  'getComputedStyle',
  'requestAnimationFrame',
  'cancelAnimationFrame',
  'sessionStorage',
] as const) {
  Object.defineProperty(globalThis, name, {
    configurable: true,
    value: dom[name],
  })
}
Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
  configurable: true,
  value: true,
})
const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { NativeSubscriptionAdmin } =
  await import('../subscriptions/native-admin')
const { NativePromotionAdmin } = await import('../promotions/native-admin')
const i18n = createInstance()
await i18n.use(initReactI18next).init({
  lng: 'en',
  resources: { en: { translation: {} } },
})
const flush = () => new Promise((resolve) => setTimeout(resolve, 20))
after(() => dom.close())

for (const entry of [
  {
    label: 'Create Plan',
    close: 'Cancel',
    field: '#native-plan-title',
    path: '/core/v1/subscriptions/admin/plans',
    data: { contract: 'core.subscriptions.v1', plans: [] },
    Component: NativeSubscriptionAdmin,
  },
  {
    label: 'Create campaign',
    close: 'Close',
    field: '#campaign-title',
    path: '/core/v1/promotions/admin/campaigns',
    data: { contract: 'core.promotions.v1', campaigns: [] },
    Component: NativePromotionAdmin,
  },
]) {
  test(`${entry.label} opens and cancels a real layout dialog without submitting`, async () => {
    const adapter = api.defaults.adapter
    const auth = useAuthStore.getState().auth
    dom.sessionStorage.clear()
    auth.setBundle({
      user: { id: 1, role: 100, username: 'synthetic-owner' },
      access_token: 'synthetic-access-token',
      token_type: 'Bearer',
      access_expires_at: 9999999999,
      session: {
        sid: 'synthetic-admin-dialog',
        current: true,
        login_method: 'password',
        ip: null,
        user_agent: null,
        created_at: 1,
        last_active_at: 1,
        expires_at: 9999999999,
      },
    })
    const requests: string[] = []
    api.defaults.adapter = async (config) => {
      requests.push(`${config.method} ${config.url}`)
      assert.equal(config.method, 'get')
      assert.equal(config.url, entry.path)
      return {
        config,
        data: entry.data,
        headers: {},
        status: 200,
        statusText: 'OK',
      }
    }
    const container = document.body.appendChild(document.createElement('div'))
    const root = createRoot(container)
    try {
      await act(async () => {
        root.render(
          <I18nextProvider i18n={i18n}>
            <entry.Component />
          </I18nextProvider>
        )
        await flush()
      })
      await act(async () => {
        await flush()
      })
      assert.equal(document.querySelector('[role="dialog"]'), null)
      const create = Array.from(container.querySelectorAll('button')).find(
        (button) => button.textContent?.trim() === entry.label
      )
      assert.ok(create)
      assert.equal(create.disabled, false)
      await act(async () => {
        create.click()
        await flush()
      })
      const dialog = document.querySelector('[role="dialog"]')
      assert.ok(dialog, 'the actual SectionPageLayout must render the dialog')
      assert.ok(dialog.querySelector(entry.field), 'creation form is reachable')
      const close = Array.from(dialog.querySelectorAll('button')).find(
        (button) => button.textContent?.trim() === entry.close
      )
      assert.ok(close)
      await act(async () => {
        close.click()
        await flush()
      })
      assert.equal(document.querySelector('[role="dialog"]'), null)
      assert.deepEqual(requests, [`get ${entry.path}`])
    } finally {
      await act(async () => root.unmount())
      container.remove()
      api.defaults.adapter = adapter
      useAuthStore.setState({ auth })
    }
  })
}
