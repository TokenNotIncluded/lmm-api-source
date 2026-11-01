/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { after, test } from 'node:test'

import { Window } from 'happy-dom'

const dom = new Window({ url: 'https://console.example.test/' })
dom.document.write('<!doctype html><html><head></head><body></body></html>')
for (const key of [
  'customElements',
  'ShadowRoot',
  'Document',
  'CSSStyleSheet',
  'window',
  'document',
  'navigator',
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
] as const) {
  Object.defineProperty(globalThis, key, {
    configurable: true,
    value: dom[key],
  })
}
Object.defineProperty(globalThis, 'matchMedia', {
  configurable: true,
  value: dom.matchMedia.bind(dom),
})
Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
  configurable: true,
  writable: true,
  value: true,
})
const { act, useEffect } = await import('react')
const { createRoot } = await import('react-dom/client')
const { QueryClient, QueryClientProvider } =
  await import('@tanstack/react-query')
const { flexRender, getCoreRowModel, useReactTable } =
  await import('@tanstack/react-table')
const { createInstance } = await import('i18next')
const { initReactI18next, I18nextProvider } = await import('react-i18next')
const { toast } = await import('sonner')
const { api } = await import('@/lib/api')
const { AxiosError } = await import('axios')
const { DEFAULT_CURRENCY_CONFIG, useSystemConfigStore } =
  await import('@/stores/system-config-store')
const { useWalletCurrencyPreferenceStore } =
  await import('@/stores/wallet-currency-preference-store')
const { TooltipProvider } = await import('@/components/ui/tooltip')
const { channelSchema } = await import('../types')
const { aggregateChannelsByTag, channelQuotaInteger } =
  await import('../lib/channel-utils')
const { handleCopyChannel } = await import('../lib/channel-actions')
const { ChannelsProvider, useChannels } = await import('./channels-provider')
const { useChannelsColumns } = await import('./channels-columns')
const { BalanceQueryDialog } = await import('./dialogs/balance-query-dialog')
type Channel = import('../types').Channel
const i18n = createInstance()
await i18n
  .use(initReactI18next)
  .init({ lng: 'en', resources: { en: { translation: {} } } })
const previousConfig = useSystemConfigStore.getState().config
const previousPreference =
  useWalletCurrencyPreferenceStore.getState().preference
useWalletCurrencyPreferenceStore.getState().setPreference('CREDIT')
useSystemConfigStore.getState().setConfig({
  currency: {
    ...DEFAULT_CURRENCY_CONFIG,
    currencyUnit: 'credit',
    quotaPerUnit: 500000,
    creditsPerUsd: 500000,
    creditsPerUsdExact: '500000',
  },
})
after(() => {
  useSystemConfigStore.setState({ config: previousConfig })
  useWalletCurrencyPreferenceStore.getState().setPreference(previousPreference)
  dom.close()
})
function channel(fields: Partial<Channel> = {}): Channel {
  return channelSchema.parse({
    id: 7,
    type: 1,
    key: 'synthetic',
    status: 1,
    name: 'Synthetic channel',
    created_time: 1,
    test_time: 0,
    response_time: 0,
    ...fields,
  })
}
function required<T>(value: T | null | undefined): T {
  assert.ok(value)
  return value
}
async function mount(row: Channel, dialog = false) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  let selected: Channel | null = null
  function Harness() {
    const { currentRow, setCurrentRow } = useChannels()
    selected = currentRow
    useEffect(() => setCurrentRow(row), [setCurrentRow])
    const columns = useChannelsColumns()
    const table = useReactTable({
      data: [row],
      columns,
      getCoreRowModel: getCoreRowModel(),
    })
    if (dialog) return <BalanceQueryDialog open onOpenChange={() => {}} />
    const cell = required(
      table
        .getRowModel()
        .rows[0]?.getAllCells()
        .find((item) => item.column.id === 'balance')
    )
    return (
      <div data-testid='balance-cell'>
        {flexRender(cell.column.columnDef.cell, cell.getContext())}
      </div>
    )
  }
  const container = document.createElement('div')
  document.body.append(container)
  const root = createRoot(container)
  await act(async () =>
    root.render(
      <QueryClientProvider client={client}>
        <I18nextProvider i18n={i18n}>
          <TooltipProvider>
            <ChannelsProvider>
              <Harness />
            </ChannelsProvider>
          </TooltipProvider>
        </I18nextProvider>
      </QueryClientProvider>
    )
  )
  return {
    selected: () => required(selected),
    async refresh() {
      const button = required(
        [...document.querySelectorAll('button')].find(
          (item) => item.textContent === 'Update Balance'
        )
      )
      await act(async () => {
        button.click()
        await new Promise((resolve) => setTimeout(resolve, 20))
      })
    },
    async cleanup() {
      await act(async () => root.unmount())
      client.clear()
      container.remove()
    },
  }
}
test('missing/null metrics stay unknown; actual zero remains visible', async () => {
  for (const fields of [
    {},
    { balance: null, used_quota: null },
    { balance: 0, used_quota: 0 },
  ]) {
    const row = channel(fields)
    assert.equal(row.balance, fields.balance)
    assert.equal(row.used_quota, fields.used_quota)
    const view = await mount(row)
    try {
      const text = required(
        document.querySelector('[data-testid=balance-cell]')
      ).textContent
      if (fields.balance === 0) {
        assert.match(text ?? '', /0/)
        assert.ok(!text?.includes('-'))
      } else assert.equal(text, '--')
    } finally {
      await view.cleanup()
    }
  }
})
test('integer strings and tag sums keep precision; unknown children make the total unknown', async () => {
  const raw = '9007199254740993'
  const row = channel({ used_quota: raw })
  assert.equal(row.used_quota, raw)
  assert.equal(channelQuotaInteger(Number.MAX_SAFE_INTEGER + 1), null)
  assert.ok(
    !channelSchema.safeParse({
      ...row,
      used_quota: Number.MAX_SAFE_INTEGER + 1,
    }).success
  )
  const view = await mount(row)
  try {
    assert.match(
      required(document.querySelector('[data-testid=balance-cell]'))
        .textContent ?? '',
      /9,007,199,254,740,993/
    )
  } finally {
    await view.cleanup()
  }
  assert.equal(
    aggregateChannelsByTag([row, channel({ used_quota: 2 })])[0]?.used_quota,
    '9007199254740995'
  )
  assert.equal(
    aggregateChannelsByTag([row, channel()])[0]?.used_quota,
    undefined
  )
  assert.equal(
    aggregateChannelsByTag([channel(), row])[0]?.used_quota,
    undefined
  )
  assert.equal(
    aggregateChannelsByTag([channel({ used_quota: 0 })])[0]?.used_quota,
    '0'
  )
})
test('refresh accepts a legacy numeric balance and uses only server observation time', async () => {
  const adapter = api.defaults.adapter
  const success = toast.success
  toast.success = () => 1
  try {
    for (const timestamp of [undefined, null, 0, -1, 1.5, 1_700_000_000]) {
      let calls = 0
      api.defaults.adapter = async (config) => {
        calls++
        assert.equal(config.url, '/api/channel/update_balance/7')
        return {
          config,
          headers: {},
          status: 200,
          statusText: 'OK',
          data: {
            success: true,
            balance: timestamp === undefined ? 12.125 : 0,
            balance_updated_time: timestamp,
          },
        }
      }
      const view = await mount(
        channel({ balance: 5, balance_updated_time: 123 }),
        true
      )
      try {
        await view.refresh()
        assert.equal(calls, 1)
        assert.equal(
          view.selected().balance,
          timestamp === undefined ? 12.125 : 0
        )
        const expected = timestamp === 1_700_000_000 ? timestamp : null
        assert.equal(view.selected().balance_updated_time, expected)
        const text = document.body.textContent ?? ''
        if (expected === null) assert.match(text, /Last updated:\s*-/)
        else {
          assert.ok(!/Last updated:\s*-/.test(text))
          assert.match(text, /2023/)
        }
      } finally {
        await view.cleanup()
      }
    }
  } finally {
    api.defaults.adapter = adapter
    toast.success = success
  }
})
test('legacy copy validates new IDs and never reports success or replays a lost receipt', async () => {
  const adapter = api.defaults.adapter
  const success = toast.success
  const error = toast.error
  let successes = 0
  let errors = 0
  toast.success = () => {
    successes++
    return 1
  }
  toast.error = () => {
    errors++
    return 1
  }
  try {
    for (const id of [
      undefined,
      null,
      0,
      -1,
      1.5,
      '8',
      7,
      Number.MAX_SAFE_INTEGER + 1,
      Infinity,
      8,
      Number.MAX_SAFE_INTEGER,
    ]) {
      let calls = 0
      const accepted: number[] = []
      api.defaults.adapter = async (config) => {
        calls++
        assert.equal(config.method, 'post')
        assert.equal(config.url, '/api/channel/copy/7')
        assert.equal(config.skipAuthRefresh, true)
        return {
          config,
          headers: {},
          status: 200,
          statusText: 'OK',
          data: { success: true, data: { id } },
        }
      }
      const before = { successes, errors }
      await handleCopyChannel(
        7,
        { suffix: '', reset_balance: true },
        undefined,
        (newId) => accepted.push(newId)
      )
      assert.equal(calls, 1)
      if (
        typeof id === 'number' &&
        Number.isSafeInteger(id) &&
        id > 0 &&
        id !== 7
      ) {
        assert.deepEqual(accepted, [id])
        assert.equal(successes, before.successes + 1)
        assert.equal(errors, before.errors)
      } else {
        assert.deepEqual(accepted, [])
        assert.equal(successes, before.successes)
        assert.equal(errors, before.errors + 1)
      }
    }
    let calls = 0
    api.defaults.adapter = async () => {
      calls++
      throw new Error('lost acknowledgement')
    }
    const before = successes
    await handleCopyChannel(7, {})
    assert.equal(calls, 1)
    assert.equal(successes, before)
    calls = 0
    api.defaults.adapter = async (config) => {
      calls++
      throw new AxiosError(
        'Unauthorized',
        'ERR_BAD_REQUEST',
        config,
        undefined,
        {
          config,
          headers: {},
          status: 401,
          statusText: 'Unauthorized',
          data: {},
        }
      )
    }
    await handleCopyChannel(7, {})
    assert.equal(calls, 1)
    assert.equal(successes, before)
  } finally {
    api.defaults.adapter = adapter
    toast.success = success
    toast.error = error
  }
})

test('legacy numeric data stays compatible while an exact provider observation owns the display and timestamp', async () => {
  const adapter = api.defaults.adapter
  const success = toast.success
  toast.success = () => 1
  const observation = {
    balance: '9007199254740993.123456789',
    unit_label: 'provider points',
    balance_updated_time: null,
    kind: 'provider',
    recorded_at: 1_700_000_000,
    source_channel_id: null,
  }
  api.defaults.adapter = async (config) => ({
    config,
    headers: {},
    status: 200,
    statusText: 'OK',
    data: { success: true, data: 12.125, observation },
  })
  const view = await mount(
    channel({ balance: 5, balance_updated_time: 123 }),
    true
  )
  try {
    await view.refresh()
    assert.equal(view.selected().balance, 12.125)
    assert.equal(view.selected().balance_updated_time, null)
    assert.equal(view.selected().observation?.balance, observation.balance)
    assert.ok(
      document.body.textContent?.includes(
        '9007199254740993.123456789 provider points'
      )
    )
    assert.match(document.body.textContent ?? '', /Last updated:\s*-/)
  } finally {
    await view.cleanup()
    api.defaults.adapter = adapter
    toast.success = success
  }
})

test('an unknown observation overrides the legacy compatibility zero in the balance cell', async () => {
  const view = await mount(
    channel({
      balance: 0,
      observation: {
        balance: null,
        unit_label: null,
        balance_updated_time: null,
        kind: 'unknown',
        recorded_at: null,
        source_channel_id: null,
      },
    })
  )
  try {
    assert.equal(
      document.querySelector('[data-testid=balance-cell]')?.textContent,
      '--'
    )
  } finally {
    await view.cleanup()
  }
})
