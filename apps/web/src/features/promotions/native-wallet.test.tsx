/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, test } from 'node:test'

import { Window } from 'happy-dom'

const dom = new Window({
  url: 'https://synthetic.example/wallet',
  width: 390,
  height: 844,
})
for (const key of [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'HTMLInputElement',
  'HTMLButtonElement',
  'SVGElement',
  'Node',
  'Element',
  'Event',
  'CustomEvent',
  'MutationObserver',
  'ResizeObserver',
  'getComputedStyle',
  'requestAnimationFrame',
  'cancelAnimationFrame',
  'sessionStorage',
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
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { useAuthStore } = await import('@/stores/auth-store')
const { redemptionDigest, rewardSchema } = await import('./native-api')
const { NativeWalletPromotions } = await import('./native-wallet')
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
const flush = () => new Promise((resolve) => setTimeout(resolve, 10))
after(() => dom.close())

function login(user = 54, sid = 'synthetic-rewards-session') {
  useAuthStore.getState().auth.setBundle({
    user: { id: user, role: 1, username: 'synthetic' },
    access_token: 'synthetic-access-token',
    token_type: 'Bearer',
    access_expires_at: 9999999999,
    session: {
      sid,
      current: true,
      login_method: 'password',
      ip: null,
      user_agent: null,
      created_at: 1,
      last_active_at: 1,
      expires_at: 9999999999,
    },
  })
}
async function enter(code: string) {
  const input = document.querySelector<HTMLInputElement>('#native-redeem-code')
  assert.ok(input)
  const set = Object.getOwnPropertyDescriptor(
    dom.HTMLInputElement.prototype,
    'value'
  )?.set
  assert.ok(set)
  await act(async () => {
    set.call(input, code)
    input.dispatchEvent(new Event('input', { bubbles: true }))
    await flush()
  })
}
async function submit() {
  const form = document.querySelector('form')
  assert.ok(form)
  await act(async () => {
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
    await flush()
    await flush()
  })
}

for (const [format, code] of [
  ['native', `lmmr_${'A'.repeat(43)}`],
  ['legacy', 'a'.repeat(32)],
]) {
  test(`redemption recovery stores only a digest, requires the same code and original key, and never replays on mount (${format})`, async () => {
    const adapter = api.defaults.adapter
    const auth = useAuthStore.getState().auth
    dom.sessionStorage.clear()
    login()
    const container = document.body.appendChild(document.createElement('div'))
    let root = createRoot(container)
    const digest = await redemptionDigest(code)
    const writes: { operation_key: string; code: string }[] = []
    let updated = 0
    api.defaults.adapter = async (config) => {
      let data: unknown
      if (config.method === 'get') {
        data = config.url?.endsWith('/campaigns')
          ? { contract: 'core.promotions.v1', campaigns: [] }
          : { contract: 'core.promotions.v1', claims: [] }
      } else {
        assert.equal(config.url, '/core/v1/promotions/redemptions')
        const raw = dom.sessionStorage.getItem(
          'lmm-commerce:54:synthetic-rewards-session:rewards'
        )
        assert.ok(raw)
        const pending = JSON.parse(raw)
        assert.equal(pending.digest, digest)
        assert.ok(
          !raw.includes(code),
          'never persist the bearer redemption code'
        )
        const request = JSON.parse(String(config.data))
        assert.equal(request.operation_key, pending.operation_key)
        assert.equal(request.code, code)
        writes.push(request)
        if (writes.length === 1) throw new Error('reward acknowledgement lost')
        data = {
          id: 'a'.repeat(64),
          campaign_id: 'b'.repeat(64),
          amount_units: '100',
          journal_id: 11,
          claimed_at: 1700000000,
        }
      }
      return { config, data, headers: {}, status: 200, statusText: 'OK' }
    }
    const render = async () => {
      await act(async () => {
        root.render(
          <I18nextProvider i18n={i18n}>
            <NativeWalletPromotions
              onUpdated={() => {
                updated++
              }}
            />
          </I18nextProvider>
        )
        await flush()
        await flush()
      })
    }
    try {
      await render()
      await enter(code)
      await submit()
      assert.equal(writes.length, 1)
      assert.ok(document.body.textContent?.includes('Result unknown.'))
      await act(async () => root.unmount())
      root = createRoot(container)
      await render()
      assert.equal(writes.length, 1)
      assert.equal(
        document.querySelector<HTMLInputElement>('#native-redeem-code')?.value,
        ''
      )
      await enter(`lmmr_${'B'.repeat(43)}`)
      await submit()
      assert.equal(
        writes.length,
        1,
        'a different code cannot replace the pending claim'
      )
      await enter(code)
      await submit()
      assert.equal(writes.length, 2)
      assert.deepEqual(writes[0], writes[1])
      assert.equal(
        dom.sessionStorage.getItem(
          'lmm-commerce:54:synthetic-rewards-session:rewards'
        ),
        null
      )
      assert.equal(updated, 1)
      const notice =
        document.querySelector('[role="status"]')?.textContent ?? ''
      assert.match(notice, /Reward received.*Core credits.*#11/)
      await act(async () => {
        login(55, 'replacement-rewards-session')
        await flush()
      })
      assert.ok(!document.body.textContent?.includes('Reward received'))
    } finally {
      await act(async () => root.unmount())
      container.remove()
      api.defaults.adapter = adapter
      useAuthStore.setState({ auth })
    }
  })
}

test('a retired reset voucher receipt cannot confirm a wallet credit', () => {
  assert.equal(
    rewardSchema.safeParse({
      reward_type: 'reset_voucher',
      reset_voucher_id: 81,
      reset_plan_id: 37,
      reset_voucher_expires_at: 1800000000,
    }).success,
    false
  )
})
