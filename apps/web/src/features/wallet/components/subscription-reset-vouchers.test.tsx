/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/
import assert from 'node:assert/strict'
import { after, afterEach, describe, test } from 'node:test'

import { AxiosError, AxiosHeaders } from 'axios'
import { Window } from 'happy-dom'
import type React from 'react'

const domWindow = new Window({
  url: 'https://console.example.test/subscriptions/reset',
})
domWindow.document.write(
  '<!doctype html><html><head></head><body></body></html>'
)
Object.defineProperty(domWindow.document, 'compatMode', {
  configurable: true,
  value: 'CSS1Compat',
})
for (const key of [
  'window',
  'document',
  'navigator',
  'history',
  'location',
  'HTMLElement',
  'HTMLButtonElement',
  'HTMLInputElement',
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
  'localStorage',
  'scrollTo',
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: domWindow[key],
  })
}

const { act } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const { createInstance } = await import('i18next')
const { I18nextProvider, initReactI18next } = await import('react-i18next')
const { api } = await import('@/lib/api')
const { SubscriptionResetVouchers } =
  await import('./subscription-reset-vouchers')

const reactTestGlobals = globalThis as typeof globalThis & {
  IS_REACT_ACT_ENVIRONMENT?: boolean
}
reactTestGlobals.IS_REACT_ACT_ENVIRONMENT = true

const i18n = createInstance()
await i18n.use(initReactI18next).init({
  lng: 'en',
  resources: { en: { translation: {} } },
})

const originalGet = api.get
const originalPost = api.post

type Deferred<T> = {
  promise: Promise<T>
  resolve: (value: T) => void
  reject: (reason?: unknown) => void
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((nextResolve, nextReject) => {
    resolve = nextResolve
    reject = nextReject
  })
  return { promise, resolve, reject }
}

async function flushQueries() {
  await new Promise((resolve) => setTimeout(resolve, 25))
}

async function waitFor(
  condition: () => boolean,
  failureMessage: string
): Promise<void> {
  for (let attempt = 0; attempt < 40; attempt += 1) {
    if (condition()) return
    await act(flushQueries)
  }
  throw new Error(`${failureMessage}: ${document.body.textContent}`)
}

function buttonNamed(container: HTMLElement, name: string) {
  return [...container.querySelectorAll<HTMLButtonElement>('button')].find(
    (button) => button.textContent?.trim() === name
  )
}

async function renderWithQuery(
  node: React.ReactNode,
  queryClient: InstanceType<typeof QueryClient>
) {
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  await act(async () => {
    root.render(
      <QueryClientProvider client={queryClient}>
        <I18nextProvider i18n={i18n}>{node}</I18nextProvider>
      </QueryClientProvider>
    )
  })
  return { container, queryClient, root }
}

async function unmount(rendered: {
  root: ReturnType<typeof createRoot>
  queryClient: InstanceType<typeof QueryClient>
}) {
  await act(async () => rendered.root.unmount())
  rendered.queryClient.clear()
}

afterEach(() => {
  api.get = originalGet
  api.post = originalPost
  document.body.replaceChildren()
})

after(() => domWindow.close())

const voucher = {
  id: 11,
  user_id: 7,
  plan_id: 3,
  plan_title: 'Pro',
  operation_id: 'reset-op',
  status: 'available' as const,
  expires_at: 1_900_000_000,
  redeemed_at: 0,
  created_at: 1_800_000_000,
}

describe('subscription reset browser accessibility', () => {
  test('keeps available vouchers visible and paginates collapsed loaded history, preserving expiry', async () => {
    const items = [
      voucher,
      ...Array.from({ length: 13 }, (_, index) => ({
        ...voucher,
        id: 100 + index,
        plan_title: `Used ${index}`,
        status: 'redeemed',
        redeemed_at: 1_800_000_100,
      })),
      {
        ...voucher,
        id: 200,
        plan_title: 'Expired boundary',
        expires_at: Math.floor(Date.now() / 1000),
      },
    ]
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false, staleTime: Infinity } },
    })
    queryClient.setQueryData(['subscription-reset-vouchers'], {
      success: true,
      data: items,
    })
    const rendered = await renderWithQuery(
      <SubscriptionResetVouchers />,
      queryClient
    )
    try {
      assert.ok(
        rendered.container.querySelector(
          '[data-slot="available-reset-vouchers"] [data-voucher-id="11"]'
        )
      )
      assert.equal(
        rendered.container.querySelector('[data-voucher-id="100"]'),
        null
      )
      const trigger = buttonNamed(
        rendered.container,
        'Show used or expired vouchers (14 loaded)'
      )
      assert.ok(trigger)
      assert.equal(trigger.tagName, 'BUTTON')
      assert.equal(trigger.getAttribute('aria-expanded'), 'false')
      trigger.focus()
      await act(async () => trigger.click())
      assert.equal(trigger.getAttribute('aria-expanded'), 'true')
      assert.equal(document.activeElement, trigger)
      const history = () =>
        rendered.container.querySelector('[data-slot="reset-voucher-history"]')
      assert.equal(history()?.querySelectorAll('[data-voucher-id]').length, 10)
      const usedRow = history()?.querySelector('[data-voucher-id="100"]')
      assert.match(usedRow?.textContent ?? '', /Expires at/)
      assert.match(usedRow?.textContent ?? '', /Redeemed at/)
      assert.equal(
        buttonNamed(history() as HTMLElement, 'Redeem reset'),
        undefined
      )
      await act(async () =>
        buttonNamed(rendered.container, 'Next page')?.click()
      )
      assert.equal(history()?.querySelectorAll('[data-voucher-id]').length, 4)
      assert.match(history()?.textContent ?? '', /Expired boundary/)
      assert.match(history()?.textContent ?? '', /Page 2 of 2/)
      await act(async () => trigger.click())
      assert.equal(trigger.getAttribute('aria-expanded'), 'false')
      await act(async () => trigger.click())
      assert.match(history()?.textContent ?? '', /Page 1 of 2/)
      await act(async () =>
        buttonNamed(rendered.container, 'Next page')?.click()
      )
      // A refreshed, shorter response must clamp the page, not show a blank list.
      await act(async () =>
        queryClient.setQueryData(['subscription-reset-vouchers'], {
          success: true,
          data: items.slice(0, 3),
        })
      )
      await waitFor(
        () => history()?.querySelectorAll('[data-voucher-id]').length === 2,
        'history page did not clamp after refresh'
      )
    } finally {
      await unmount(rendered)
    }
  })

  for (const succeeds of [true, false]) {
    test(`moves a voucher to history only after successful redemption (${succeeds})`, async () => {
      const queryClient = new QueryClient({
        defaultOptions: { queries: { retry: false, staleTime: Infinity } },
      })
      queryClient.setQueryData(['subscription-reset-vouchers'], {
        success: true,
        data: [voucher],
      })
      const refresh = deferred<{
        data: { success: boolean; data: (typeof voucher)[] }
      }>()
      api.get = (async () => refresh.promise) as typeof api.get
      api.post = (async () => ({
        data: succeeds
          ? { success: true, data: { reset_count: 1, restored_quota: 10 } }
          : { success: false, message: 'fixture redemption failure' },
      })) as typeof api.post
      let redemptions = 0
      const rendered = await renderWithQuery(
        <SubscriptionResetVouchers
          onRedeemed={() => {
            redemptions++
          }}
        />,
        queryClient
      )
      try {
        await act(async () =>
          buttonNamed(rendered.container, 'Redeem reset')?.click()
        )
        const dialog = document.querySelector<HTMLElement>(
          '[role="alertdialog"]'
        )
        assert.ok(dialog)
        await act(async () => buttonNamed(dialog, 'Redeem reset')?.click())
        if (succeeds) {
          await waitFor(
            () =>
              buttonNamed(
                rendered.container,
                'Show used or expired vouchers (1 loaded)'
              ) != null,
            'redeemed voucher did not move to collapsed history'
          )
          assert.equal(
            rendered.container.querySelector(
              '[data-slot="available-reset-vouchers"] [data-voucher-id]'
            ),
            null
          )
          assert.equal(redemptions, 1)
          await act(async () =>
            buttonNamed(
              rendered.container,
              'Show used or expired vouchers (1 loaded)'
            )?.click()
          )
          assert.match(
            rendered.container.querySelector(
              '[data-slot="reset-voucher-history"]'
            )?.textContent ?? '',
            /Expires at/
          )
          await act(async () =>
            refresh.resolve({ data: { success: true, data: [voucher] } })
          )
          await waitFor(
            () => buttonNamed(rendered.container, 'Refresh') != null,
            'refetch did not settle'
          )
          assert.equal(
            rendered.container.querySelector(
              '[data-slot="available-reset-vouchers"] [data-voucher-id]'
            ),
            null
          )
        } else {
          await waitFor(
            () => buttonNamed(dialog, 'Redeem reset')?.disabled === false,
            'failed redemption did not become retryable'
          )
          assert.ok(
            rendered.container.querySelector(
              '[data-slot="available-reset-vouchers"] [data-voucher-id="11"]'
            )
          )
          assert.equal(redemptions, 0)
          assert.equal(
            rendered.container.querySelector(
              '[data-slot="collapsible-trigger"]'
            ),
            null
          )
        }
      } finally {
        await unmount(rendered)
      }
    })
  }
})

test('an open voucher confirmation expires without a query, and a click before the wakeup still cannot redeem', async () => {
  const realNow = Date.now
  let now = 1_900_000_000_000
  Date.now = () => now
  const expiresAt = now / 1000 + 5
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: Infinity } },
  })
  queryClient.setQueryData(['subscription-reset-vouchers'], {
    success: true,
    data: [{ ...voucher, expires_at: expiresAt }],
  })
  let reads = 0
  let writes = 0
  api.get = (async () => {
    reads++
    throw new Error('unexpected expiry refetch')
  }) as typeof api.get
  api.post = (async () => {
    writes++
    throw new Error('expired voucher must not be sent')
  }) as typeof api.post
  const rendered = await renderWithQuery(
    <SubscriptionResetVouchers />,
    queryClient
  )
  try {
    await act(async () =>
      buttonNamed(rendered.container, 'Redeem reset')?.click()
    )
    const dialog = document.querySelector<HTMLElement>('[role="alertdialog"]')
    assert.ok(dialog)
    const confirm = buttonNamed(dialog, 'Redeem reset')
    assert.ok(confirm)
    now = expiresAt * 1000 - 1
    await act(async () => window.dispatchEvent(new Event('focus')))
    assert.equal(confirm.disabled, false)
    now = expiresAt * 1000
    // The deadline event has not fired yet: the handler must check fresh time.
    await act(async () => confirm.click())
    assert.equal(writes, 0)
    await act(async () => document.dispatchEvent(new Event('visibilitychange')))
    assert.equal(confirm.disabled, true)
    assert.equal(
      rendered.container.querySelector(
        '[data-slot="available-reset-vouchers"] [data-voucher-id="11"]'
      ),
      null
    )
    assert.equal(
      reads,
      0,
      'deadline notifications do not change query keys or refetch data'
    )
  } finally {
    await unmount(rendered)
    Date.now = realNow
  }
})

function unavailableVoucherRoute() {
  const config = {
    headers: new AxiosHeaders(),
    url: '/api/subscription/self/reset-vouchers',
    method: 'get',
  }
  return new AxiosError(
    'Route not found',
    'ERR_BAD_REQUEST',
    config,
    undefined,
    {
      config,
      status: 404,
      statusText: 'Not Found',
      headers: {},
      data: '',
    }
  )
}

test('an older Core shows unsupported resets without retries or empty-wallet claims and refresh can discover support', async () => {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: 3, retryDelay: 0 } },
  })
  let reads = 0
  let writes = 0
  api.get = (async (url, config) => {
    assert.equal(url, '/api/subscription/self/reset-vouchers')
    assert.equal(config?.skipErrorHandler, true)
    reads++
    if (reads === 1) throw unavailableVoucherRoute()
    return { data: { success: true, data: [voucher] } }
  }) as typeof api.get
  api.post = (async () => {
    writes++
    throw new Error('Availability detection must not redeem a voucher')
  }) as typeof api.post
  const rendered = await renderWithQuery(
    <SubscriptionResetVouchers />,
    queryClient
  )
  const unsupported =
    'Banked subscription resets are not available on this server.'
  try {
    await waitFor(
      () => rendered.container.textContent?.includes(unsupported) === true,
      'missing route did not show unsupported availability'
    )
    await act(flushQueries)
    assert.equal(
      reads,
      1,
      'an absent endpoint must not be automatically retried'
    )
    assert.equal(rendered.container.querySelector('[role="alert"]'), null)
    assert.equal(
      rendered.container.textContent?.includes('No banked reset vouchers'),
      false
    )
    assert.equal(buttonNamed(rendered.container, 'Redeem reset'), undefined)
    assert.equal(writes, 0)
    await act(async () => buttonNamed(rendered.container, 'Refresh')?.click())
    await waitFor(
      () => buttonNamed(rendered.container, 'Redeem reset') != null,
      'a supported server response did not restore the voucher list'
    )
    assert.equal(reads, 2)
    assert.equal(rendered.container.textContent?.includes(unsupported), false)
    assert.ok(rendered.container.querySelector('[data-voucher-id="11"]'))
    assert.equal(writes, 0)
  } finally {
    await unmount(rendered)
  }
})

test('a missing voucher route clears cached redemption controls and closes an open confirmation', async () => {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: Infinity } },
  })
  queryClient.setQueryData(['subscription-reset-vouchers'], {
    success: true,
    data: [voucher],
  })
  let reads = 0
  let writes = 0
  api.get = (async () => {
    reads++
    if (reads === 1) throw unavailableVoucherRoute()
    return { data: { success: true, data: [voucher] } }
  }) as typeof api.get
  api.post = (async () => {
    writes++
    throw new Error('Unsupported voucher actions must not be sent')
  }) as typeof api.post
  const rendered = await renderWithQuery(
    <SubscriptionResetVouchers />,
    queryClient
  )
  try {
    await act(async () =>
      buttonNamed(rendered.container, 'Redeem reset')?.click()
    )
    assert.ok(document.querySelector('[role="alertdialog"]'))
    await act(async () =>
      queryClient.refetchQueries({ queryKey: ['subscription-reset-vouchers'] })
    )
    await waitFor(
      () => document.querySelector('[role="alertdialog"]') === null,
      'unsupported server retained a stale redemption dialog'
    )
    assert.equal(rendered.container.querySelector('[data-voucher-id]'), null)
    assert.equal(buttonNamed(rendered.container, 'Redeem reset'), undefined)
    assert.equal(reads, 1)
    assert.equal(writes, 0)
    await act(async () => buttonNamed(rendered.container, 'Refresh')?.click())
    await waitFor(
      () => rendered.container.querySelector('[data-voucher-id="11"]') != null,
      'supported refresh did not restore the voucher'
    )
    assert.equal(reads, 2)
    assert.equal(document.querySelector('[role="alertdialog"]'), null)
    assert.equal(
      writes,
      0,
      'restoring support must not reopen or redeem an old confirmation'
    )
  } finally {
    await unmount(rendered)
  }
})
