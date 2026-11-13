/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'

import { z } from 'zod'

import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

import {
  discountBatchSchema,
  getDiscountPage,
  nativeDiscountSchema,
  saveDiscount,
  setDiscountEnabled,
  removeDiscount,
  generateDiscountBatch,
} from './native-api'
import {
  discountDefinition,
  editDiscountForm,
  emptyDiscountForm,
} from './native-form'

const previousAuth = useAuthStore.getState().auth
const previousAdapter = api.defaults.adapter
const scope = { userId: 7401, sessionId: 'native-discount-admin' }
const definition = {
  code: 'SAVE20',
  name: 'Native credits',
  discount_percent: 20,
  min_credit_units: '9007199254740993',
  max_uses: '9223372036854775807',
  enabled: true,
  starts_at: '0',
  ends_at: '0',
}
const code = nativeDiscountSchema.parse({
  ...definition,
  id: '9007199254740993',
  used_count: '0',
  reserved_count: '1',
})
function login(sid = scope.sessionId) {
  useAuthStore.getState().auth.setBundle({
    user: { id: scope.userId, role: 1000, username: 'synthetic-root' },
    access_token: 'synthetic-native-discount-session',
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
afterEach(() => {
  api.defaults.adapter = previousAdapter
  useAuthStore.setState({ auth: previousAuth })
})

test('native administrator forms preserve i64 values and local date windows without fiat conversion', () => {
  assert.deepEqual(discountDefinition(editDiscountForm(code)), definition)
  assert.equal(
    discountDefinition({
      ...emptyDiscountForm,
      code: ' save20 ',
      name: 'Example',
      minimum: definition.min_credit_units,
    }).min_credit_units,
    definition.min_credit_units
  )
  const dated = nativeDiscountSchema.parse({
    ...code,
    starts_at: '1794500001',
    ends_at: '1794600002',
  })
  assert.deepEqual(discountDefinition(editDiscountForm(dated)), {
    ...definition,
    starts_at: dated.starts_at,
    ends_at: dated.ends_at,
  })
  for (const value of ['9223372036854775808', '1.5', '1e6', '-1', 'invalid']) {
    assert.throws(() =>
      discountDefinition({
        ...emptyDiscountForm,
        code: 'SAVE20',
        name: 'Example',
        minimum: value,
      })
    )
  }
  assert.equal(
    nativeDiscountSchema.safeParse({ ...code, id: 'invalid' }).success,
    false
  )
  assert.equal(
    nativeDiscountSchema.safeParse({ ...code, id: Number.MAX_SAFE_INTEGER + 1 })
      .success,
    false
  )
  assert.equal(
    nativeDiscountSchema.safeParse({
      ...code,
      max_uses: Number.MAX_SAFE_INTEGER + 1,
    }).success,
    false
  )
})

test('native management requests preserve cursors, exact quantities and isolated status writes', async () => {
  login()
  const writes: { method: string; path: string; data: unknown }[] = []
  api.defaults.adapter = async (config) => {
    const path = config.url ?? ''
    let data: unknown
    if (config.data !== undefined) {
      assert.equal(typeof config.data, 'string')
      data = JSON.parse(config.data)
    }
    writes.push({ method: config.method ?? '', path, data })
    const payload =
      config.method === 'get'
        ? { items: [code], next_after: null }
        : config.method === 'delete'
          ? null
          : path.endsWith('/batch')
            ? { items: [code] }
            : code
    return {
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: { success: true, data: payload },
    }
  }
  const page = await getDiscountPage(scope, '9007199254740993', {
    keyword: 'SAVE & 20',
    enabled: 'false',
  })
  assert.equal(page.items[0].id, code.id)
  const query = new URL(writes[0].path, 'https://synthetic.invalid')
    .searchParams
  assert.equal(query.get('after_id'), code.id)
  assert.equal(query.get('keyword'), 'SAVE & 20')
  assert.equal(query.get('enabled'), 'false')
  await saveDiscount(scope, definition, code)
  await setDiscountEnabled(scope, code, false)
  assert.deepEqual(writes[1].data, definition)
  assert.deepEqual(writes[2].data, { enabled: false })
  const batch = discountBatchSchema.parse({
    ...definition,
    code: undefined,
    count: 1,
    request_key: crypto.randomUUID(),
  })
  await generateDiscountBatch(scope, batch)
  await generateDiscountBatch(scope, batch)
  assert.deepEqual(writes[3].data, writes[4].data)
  assert.equal(
    z.object({ min_credit_units: z.string() }).parse(writes[3].data)
      .min_credit_units,
    definition.min_credit_units
  )
  await removeDiscount(scope, code)
  assert.equal(writes[5].method, 'delete')
  assert.ok(writes[5].path.endsWith(code.id))
})

test('an administrator response from a previous login cannot enter the current workspace', async () => {
  login()
  api.defaults.adapter = async (config) => {
    login('different-session')
    return {
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: { success: true, data: { items: [code], next_after: null } },
    }
  }
  await assert.rejects(
    getDiscountPage(scope, '0', { keyword: '', enabled: 'all' })
  )
})

test('editing a name preserves the second occurrence of a clock time during DST rollback', () => {
  const previousTimezone = process.env.TZ
  try {
    process.env.TZ = 'America/New_York'
    const starts_at = Math.floor(
      Date.parse('2025-11-02T06:30:00Z') / 1000
    ).toString()
    const original = nativeDiscountSchema.parse({ ...code, starts_at })
    const form = editDiscountForm(original)
    assert.equal(
      discountDefinition({ ...form, name: 'New name' }).starts_at,
      starts_at
    )
  } finally {
    if (previousTimezone === undefined) delete process.env.TZ
    else process.env.TZ = previousTimezone
  }
})

test('the native management page renders its search controls and fetched code records', async () => {
  const { Window } = await import('happy-dom')
  const dom = new Window({ url: 'https://native-admin.example' })
  const previousGlobals = new Map<string, PropertyDescriptor | undefined>()
  for (const key of [
    'matchMedia',
    'ResizeObserver',
    'IntersectionObserver',
    'HTMLButtonElement',
    'HTMLSelectElement',
    'HTMLTextAreaElement',
    'DocumentFragment',
    'FocusEvent',
    'MouseEvent',
    'KeyboardEvent',
    'PointerEvent',
    'window',
    'document',
    'navigator',
    'HTMLElement',
    'HTMLInputElement',
    'SVGElement',
    'Node',
    'Element',
    'Event',
    'CustomEvent',
    'MutationObserver',
    'getComputedStyle',
    'requestAnimationFrame',
    'cancelAnimationFrame',
    'sessionStorage',
  ]) {
    previousGlobals.set(key, Object.getOwnPropertyDescriptor(globalThis, key))
    Object.defineProperty(globalThis, key, {
      configurable: true,
      value: Reflect.get(dom, key),
    })
  }
  previousGlobals.set(
    'IS_REACT_ACT_ENVIRONMENT',
    Object.getOwnPropertyDescriptor(globalThis, 'IS_REACT_ACT_ENVIRONMENT')
  )
  Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
    configurable: true,
    value: true,
  })
  const { act, createElement } = await import('react')
  const { createRoot } = await import('react-dom/client')
  const { createInstance } = await import('i18next')
  const { I18nextProvider, initReactI18next } = await import('react-i18next')
  const { NativeDiscountCodes } = await import('./native-discount-codes')
  const i18n = createInstance()
  await i18n
    .use(initReactI18next)
    .init({ lng: 'en', resources: { en: { translation: {} } } })
  login()
  const container = document.body.appendChild(document.createElement('div'))
  const root = createRoot(container)
  api.defaults.adapter = async (config) => ({
    config,
    status: 200,
    statusText: 'OK',
    headers: {},
    data: { success: true, data: { items: [code], next_after: null } },
  })
  try {
    await act(async () => {
      root.render(
        createElement(
          I18nextProvider,
          { i18n },
          createElement(NativeDiscountCodes)
        )
      )
      await new Promise((resolve) => setTimeout(resolve, 20))
    })
    assert.ok(container.querySelector('#native-discount-search'))
    assert.ok(container.textContent?.includes(code.code))
    assert.ok(container.textContent?.includes(code.name))
    assert.ok(
      Array.from(container.querySelectorAll('button')).some(
        (button) => button.textContent === 'Next'
      )
    )
  } finally {
    await act(async () => root.unmount())
    container.remove()
    for (const [key, descriptor] of previousGlobals) {
      if (descriptor) Object.defineProperty(globalThis, key, descriptor)
      else Reflect.deleteProperty(globalThis, key)
    }
    await dom.happyDOM.abort()
  }
})
