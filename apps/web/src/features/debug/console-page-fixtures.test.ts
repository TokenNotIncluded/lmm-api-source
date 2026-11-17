/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.

For commercial licensing, please contact support@quantumnous.com
*/
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { AxiosHeaders, type InternalAxiosRequestConfig } from 'axios'
import { Window } from 'happy-dom'

import { hasCompletePublicCreditCatalog } from '@/features/wallet/lib/topup-credit-metadata'

const domWindow = new Window({ url: 'http://127.0.0.1:4174/' })
Object.defineProperty(globalThis, 'window', {
  configurable: true,
  value: domWindow,
})
const { consolePageFixture, withConsolePageFixtures } =
  await import('./console-page-fixtures')
const { useAuthStore } = await import('@/stores/auth-store')
const config = (url: string, method = 'get') =>
  ({ url, method, headers: new AxiosHeaders() }) as InternalAxiosRequestConfig

test('native admin review data validates, filters users and never handles writes or ordinary-user reads', async () => {
  const { adminReviewFixture } = await import('./admin-review-fixtures')
  const {
    nativeGroupsResponseSchema,
    nativeUserDetailSchema,
    nativeUsersResponseSchema,
    DEFAULT_NATIVE_USERS_QUERY,
  } = await import('@/features/native-users/contract')
  const previous = useAuthStore.getState().auth.user
  try {
    useAuthStore
      .getState()
      .auth.setUser({ id: 9001, username: 'review', role: 10 })
    const request = {
      ...config('/core/v1/admin/users'),
      params: { ...DEFAULT_NATIVE_USERS_QUERY },
    }
    // The client omits an unset team ID instead of sending an empty integer.
    delete (request.params as Partial<typeof DEFAULT_NATIVE_USERS_QUERY>)
      .team_id
    const users = nativeUsersResponseSchema.parse(adminReviewFixture(request))
    assert.equal(users.items.length, 3)
    const teamUsers = nativeUsersResponseSchema.parse(
      adminReviewFixture({
        ...request,
        params: { ...request.params, team_id: '12' },
      })
    )
    assert.ok(teamUsers.items.length > 0)
    assert.ok(
      teamUsers.items.every((user) =>
        user.teams.some((team) => team.id === '12')
      )
    )
    const absentTeam = nativeUsersResponseSchema.parse(
      adminReviewFixture({
        ...request,
        params: { ...request.params, team_id: '99999' },
      })
    )
    assert.equal(absentTeam.total, 0)
    assert.deepEqual(absentTeam.items, [])

    const groups = nativeGroupsResponseSchema.parse(
      adminReviewFixture(config('/core/v1/admin/users/groups'))
    )
    assert.deepEqual(groups.items, ['alpha', 'default', 'team-beta'])
    const secondGroupsPage = nativeGroupsResponseSchema.parse(
      adminReviewFixture({
        ...config('/core/v1/admin/users/groups'),
        params: { page: 2, page_size: 20 },
      })
    )
    assert.equal(secondGroupsPage.page, 2)
    assert.equal(secondGroupsPage.total, 3)
    assert.deepEqual(secondGroupsPage.items, [])
    for (const user of users.items) {
      const detail = nativeUserDetailSchema.parse(
        adminReviewFixture(config(`/core/v1/admin/users/${user.id}`))
      )
      assert.deepEqual(detail.user, user)
      assert.equal(detail.statistics?.user_id, user.id)
      assert.equal(
        detail.statistics?.personal_account_id,
        user.personal_account_id
      )
      assert.equal(detail.statistics?.usage.status, 'available')
      assert.equal(detail.statistics?.historical_counters.status, 'available')
      assert.equal(detail.statistics?.referrals.invited_users, '12')
      assert.equal(detail.usage.status, 'available')
      if (detail.usage.status === 'available') {
        assert.equal(detail.usage.net_consumption_units, '1000')
        assert.equal(detail.usage.historical_imports_included, false)
      }
    }
    for (const id of ['0', '01006', '100', '10060', 'missing']) {
      assert.equal(
        adminReviewFixture(config(`/core/v1/admin/users/${id}`)),
        undefined
      )
    }
    const { withUsageLogsReviewFixtures } =
      await import('./usage-logs-review-fixtures')
    const logs = await withUsageLogsReviewFixtures(async () => {
      throw new Error('unmocked')
    })(config('/core/v1/usage/admin/logs'))
    assert.ok(logs.data.data.items.length > 0)
    for (const row of logs.data.data.items) {
      const detail = nativeUserDetailSchema.parse(
        adminReviewFixture(config(`/core/v1/admin/users/${row.user_id}`))
      )
      assert.equal(detail.user.username, row.username)
    }
    request.params.status = 'disabled'
    const filtered = nativeUsersResponseSchema.parse(
      adminReviewFixture(request)
    )
    assert.equal(filtered.total, 1)
    assert.equal(filtered.items[0].active, false)
    request.params.keyword = 'no-synthetic-match'
    assert.equal(
      nativeUsersResponseSchema.parse(adminReviewFixture(request)).total,
      0
    )
    for (const path of [
      '/api/extensions/plugins',
      '/api/channel/ops',
      '/core/v1/admin/routing/configuration',
      '/core/v1/admin/users',
      '/core/v1/admin/users/groups',
      '/core/v1/admin/users/1006',
      '/core/v1/admin/users/9007199254740993',
    ]) {
      assert.ok(adminReviewFixture(config(path)))
      assert.equal(adminReviewFixture(config(path, 'post')), undefined)
      assert.equal(
        adminReviewFixture(config(`https://example.invalid${path}`)),
        undefined
      )
    }
    useAuthStore
      .getState()
      .auth.setUser({ id: 9001, username: 'review', role: 1 })
    assert.equal(
      adminReviewFixture(config('/api/extensions/plugins')),
      undefined
    )
    assert.equal(adminReviewFixture(config('/core/v1/admin/users')), undefined)
    assert.equal(
      adminReviewFixture(config('/core/v1/admin/users/groups')),
      undefined
    )
    assert.equal(
      adminReviewFixture(config('/core/v1/admin/users/1006')),
      undefined
    )
  } finally {
    useAuthStore.getState().auth.setUser(previous)
  }
})

test('review fixtures never handle writes, unknown paths, or remote requests', async () => {
  assert.equal(consolePageFixture(config('/api/token/', 'post')), undefined)
  assert.equal(
    consolePageFixture(config('https://example.invalid/api/token/')),
    undefined
  )
  assert.equal(
    consolePageFixture(config('/api/not-in-the-explicit-catalog')),
    undefined
  )
  let passed = 0
  const wrapped = withConsolePageFixtures(async (request) => {
    passed += 1
    throw new Error(`blocked:${request.url}`)
  })
  await assert.rejects(
    wrapped(config('/api/hero-sms/sms/orders', 'post')),
    /blocked/
  )
  assert.equal(passed, 1)
  const response = await wrapped(config('/api/token/'))
  assert.equal(response.status, 200)
  assert.equal(passed, 1)
})

test('empty read responses are cloned so one page cannot mutate another fixture', () => {
  const first = consolePageFixture(config('/api/user/oauth/bindings')) as {
    data: unknown[]
  }
  first.data.push('local mutation')
  const second = consolePageFixture(config('/api/user/oauth/bindings')) as {
    data: unknown[]
  }
  assert.deepEqual(second.data, [])
})

test('store and security consumers receive the real controller success shapes', async () => {
  const { storeApi } = await import('@/features/store/api')
  const { marketAIReviewAPI } = await import('@/features/market-ai-review/api')
  const { listModerationAppeals } =
    await import('@/features/system-settings/security/security-audit-api')
  const { api } = await import('@/lib/api')
  const previousUser = useAuthStore.getState().auth.user
  const previousAdapter = api.defaults.adapter
  useAuthStore
    .getState()
    .auth.setUser({ id: 9001, username: 'review', role: 10 })
  api.defaults.adapter = withConsolePageFixtures(async () => {
    throw new Error('unmocked')
  })
  try {
    const store = await storeApi.config()
    const basis = store as typeof store & {
      credits_per_usd: number
      external_minimum_quota: number
    }
    assert.equal(store.fee_bps, 100)
    assert.equal(store.promotion_quota, 500000)
    assert.equal(store.minimum_unit_price_quota, 500000)
    assert.equal(basis.credits_per_usd, 500000)
    assert.equal(basis.external_minimum_quota, 5000000)
    assert.equal(store.linuxdo_units_per_usd, '')
    assert.equal(store.disclaimer_version, 'merchant-store-v1')
    assert.ok(store.disclaimer_text.length > 0)
    assert.deepEqual(store.product_link_presets, [])
    assert.deepEqual(store.platform_payment_methods, [
      {
        provider: 'balance',
        payment_type: 'balance',
        name: 'Platform balance',
        supported: true,
        configured: true,
      },
    ])
    assert.deepEqual(
      store.platform_payment_catalog,
      store.platform_payment_methods
    )
    const settings = await marketAIReviewAPI.settings()
    assert.equal(settings.tool_mode, 'off')
    assert.equal(settings.store_mode, 'off')
    assert.equal(settings.review_group, 'default')
    assert.equal(settings.review_model, 'omni-moderation-latest')
    assert.equal(settings.engine, 'openai_moderation')
    assert.deepEqual(settings.supported_inputs, ['text'])
    assert.equal(settings.categories.length, 13)
    assert.ok(settings.categories.includes('violence/graphic'))
    const appeals = await listModerationAppeals()
    assert.deepEqual(appeals, { success: true, message: '', data: [] })
    settings.categories.push('mutation')
    assert.equal((await marketAIReviewAPI.settings()).categories.length, 13)
  } finally {
    api.defaults.adapter = previousAdapter
    useAuthStore.getState().auth.setUser(previousUser)
  }
})

test('the three new reads keep exact paths, GET methods and admin permissions', async () => {
  const previousUser = useAuthStore.getState().auth.user
  const wrapped = withConsolePageFixtures(async () => {
    throw new Error('unmocked')
  })
  const paths = [
    '/api/store/config',
    '/api/security/market-ai-review/settings',
    '/api/security/admin/violation-fee-appeals',
  ]
  try {
    for (const role of [0, 1, 9]) {
      useAuthStore
        .getState()
        .auth.setUser({ id: 9002, username: 'review', role })
      assert.equal((await wrapped(config(paths[0]))).data.success, true)
      for (const path of paths.slice(1)) {
        await assert.rejects(wrapped(config(path)), /unmocked/, path)
      }
    }
    useAuthStore.getState().auth.setUser(null)
    for (const path of paths.slice(1)) {
      await assert.rejects(wrapped(config(path)), /unmocked/, path)
    }
    for (const role of [10, 100]) {
      useAuthStore
        .getState()
        .auth.setUser({ id: 9001, username: 'review', role })
      for (const path of paths) {
        assert.equal((await wrapped(config(path))).data.success, true)
        for (const method of ['post', 'put', 'patch', 'delete']) {
          await assert.rejects(wrapped(config(path, method)), /unmocked/, path)
        }
        for (const url of [
          `${path}/unlisted`,
          `https://example.invalid${path}`,
          `http://credential@127.0.0.1:4174${path}`,
        ]) {
          await assert.rejects(wrapped(config(url)), /unmocked/, url)
        }
      }
    }
    for (const path of [
      '/api/store/orders',
      '/api/store/guest/orders',
      '/api/store/orders/order-preview/pay',
      '/api/store/orders/order-preview/email',
      '/api/security/admin/violation-fee-appeals/1/approve',
      '/api/security/not-in-the-explicit-catalog',
    ]) {
      await assert.rejects(wrapped(config(path, 'post')), /unmocked/, path)
    }
  } finally {
    useAuthStore.getState().auth.setUser(previousUser)
  }
})

test('admin list fixtures keep the API array contract rather than a paginated envelope', () => {
  for (const url of [
    '/api/red-packet/admin',
    '/api/assistant/admin/registration-events',
  ]) {
    const response = consolePageFixture(config(url)) as { data: unknown }
    assert.ok(Array.isArray(response.data), url)
  }
})

test('assistant model reads are explicit, cloned fixtures and never authorize writes', () => {
  const first = consolePageFixture(
    config('/api/assistant/models?group=default')
  ) as { data: string[] }
  assert.ok(Array.isArray(first.data))
  assert.ok(first.data.length > 0)
  assert.ok(first.data.every((model) => typeof model === 'string'))
  first.data.push('mutated-preview')
  const second = consolePageFixture(
    config('/api/assistant/models?group=default')
  ) as { data: string[] }
  assert.ok(!second.data.includes('mutated-preview'))
  assert.equal(
    consolePageFixture(config('/api/assistant/models', 'post')),
    undefined
  )
})

test('wallet review exposes versioned raw Credits and correct Alipay CNY limits without payment writes', async () => {
  const response = consolePageFixture(config('/api/user/topup/info')) as {
    data: {
      enable_online_topup: boolean
      payment_available: boolean
      amount_unit: string
      credit_metadata_available: boolean
      credit_metadata_version: number
      credit_amount_options: number[]
      credit_discount: Record<string, number>
      credit_min_topup: number
      stripe_credit_min_topup: number
      waffo_credit_min_topup: number
      pancake_credit_min_topup: number
      stripe_credit_max_topup: number | null
      waffo_credit_max_topup: number | null
      pancake_credit_max_topup: number | null
      pay_methods: Array<Record<string, unknown>>
    }
  }
  assert.equal(response.data.enable_online_topup, true)
  assert.equal(response.data.payment_available, true)
  assert.equal(response.data.pay_methods[0]?.type, 'alipay')
  assert.equal(response.data.amount_unit, 'LEGACY')
  assert.equal(response.data.credit_metadata_available, true)
  assert.equal(response.data.credit_metadata_version, 1)
  assert.deepEqual(
    response.data.credit_amount_options,
    [5000000, 25000000, 50000000, 100000000]
  )
  assert.deepEqual(response.data.credit_discount, { 50000000: 0.9 })
  assert.ok(hasCompletePublicCreditCatalog(response.data))
  assert.equal(response.data.credit_min_topup, 500000)
  assert.equal(response.data.stripe_credit_min_topup, 500000)
  assert.equal(response.data.waffo_credit_min_topup, 0)
  assert.equal(response.data.pancake_credit_min_topup, 0)
  assert.equal(response.data.stripe_credit_max_topup, 5000000000)
  assert.equal(response.data.waffo_credit_max_topup, null)
  assert.equal(response.data.pancake_credit_max_topup, null)
  assert.equal(response.data.pay_methods[0]?.min_topup_credit, '3500000')
  assert.equal(response.data.pay_methods[0]?.max_topup_credit, '350000000')
  assert.equal(response.data.pay_methods[0]?.min_topup_unit, 'USD')
  assert.equal(response.data.pay_methods[0]?.min_topup, 7)
  assert.equal(response.data.pay_methods[0]?.max_topup, '700')
  assert.equal(response.data.pay_methods[0]?.settlement_currency, 'CNY')
  assert.equal(response.data.pay_methods[0]?.platform_units_per_usd, 7)
  assert.equal(response.data.pay_methods[0]?.settlement_units_per_usd, 7)
  assert.equal(
    response.data.pay_methods[0]?.settlement_units_per_platform_unit,
    1
  )
  const wrapped = withConsolePageFixtures(async () => {
    throw new Error('blocked')
  })
  for (const path of [
    '/api/user/topup',
    '/api/user/pay',
    '/api/user/stripe/pay',
    '/api/user/topup/currency/pay',
    '/api/user/topup/currency/stripe/pay',
    '/api/user/topup/currency/waffo/pay',
    '/api/user/topup/currency/waffo-pancake/pay',
    '/api/user/topup/currency/v2/pay',
    '/api/user/topup/currency/v2/stripe/pay',
    '/api/user/topup/currency/v2/waffo/pay',
    '/api/user/topup/currency/v2/waffo-pancake/pay',
    '/api/subscription/balance/pay',
  ]) {
    await assert.rejects(wrapped(config(path, 'post')), /blocked/, path)
  }
})

test('usage review windows filter timestamps without duplicating model totals', () => {
  const stamp = 1790035200
  const first = config(
    `/api/data/self?start_timestamp=${stamp - 30 * 86400}&end_timestamp=${stamp - 3 * 86400}`
  )
  const second = config('/api/data/self')
  second.params = {
    start_timestamp: stamp - 3 * 86400 + 1,
    end_timestamp: stamp,
  }
  type UsageRow = {
    model_name: string
    token_used: number
    count: number
    quota: number
    created_at: number
  }
  const rows = [first, second].flatMap(
    (request) => (consolePageFixture(request) as { data: UsageRow[] }).data
  )
  assert.equal(rows.length, 7)
  assert.equal(new Set(rows.map((row) => row.created_at)).size, 7)
  assert.equal(
    rows.reduce((total, row) => total + row.token_used, 0),
    24780
  )
  assert.equal(
    rows.reduce((total, row) => total + row.count, 0),
    98
  )
  assert.ok(rows.every((row) => row.model_name && row.quota > 0))
})

test('overview review data preserves currency separation and admin-only reads', async () => {
  const { getAdminSiteStatistics } =
    await import('@/features/dashboard/site-statistics')
  const { api } = await import('@/lib/api')
  const previousUser = useAuthStore.getState().auth.user
  const previousAdapter = api.defaults.adapter
  api.defaults.adapter = withConsolePageFixtures(async () => {
    throw new Error('unmocked')
  })
  try {
    for (const role of [1, 2, 11]) {
      useAuthStore
        .getState()
        .auth.setUser({ id: 9001, username: 'review', role })
      assert.equal(
        consolePageFixture(config('/api/finance/site-statistics')),
        undefined
      )
    }
    for (const role of [10, 100]) {
      useAuthStore
        .getState()
        .auth.setUser({ id: 9001, username: 'review', role })
      const data = await getAdminSiteStatistics()
      assert.deepEqual(
        data.recharge.currencies.map((row) => row.currency),
        ['CNY', 'USD']
      )
      assert.deepEqual(
        data.recharge.virtual_units.map((row) => row.currency),
        ['LDC']
      )
      assert.equal(data.recharge.unconfirmed_orders, 4)
      data.recharge.currencies.length = 0
      assert.equal(
        (await getAdminSiteStatistics()).recharge.currencies.length,
        2
      )
    }
    assert.equal(
      consolePageFixture(config('/api/finance/site-statistics', 'post')),
      undefined
    )
    assert.equal(
      consolePageFixture(
        config('http://user:pass@127.0.0.1:4174/api/finance/site-statistics')
      ),
      undefined
    )
  } finally {
    api.defaults.adapter = previousAdapter
    useAuthStore.getState().auth.setUser(previousUser)
  }
})

test('native operations preview follows real read contracts and retains administrator-only access', async () => {
  const { getNativeSystemResources, listSystemInstances } =
    await import('@/features/system-info/api')
  const { api } = await import('@/lib/api')
  const previousUser = useAuthStore.getState().auth.user
  const previousAdapter = api.defaults.adapter
  const wrapped = withConsolePageFixtures(async () => {
    throw new Error('unmocked')
  })
  api.defaults.adapter = wrapped
  const paths = ['/core/v1/admin/instances', '/core/v1/admin/system-resources']
  try {
    useAuthStore
      .getState()
      .auth.setUser({ id: 9001, username: 'review', role: 10 })
    const instances = await listSystemInstances()
    assert.equal(instances.data?.length, 2)
    assert.equal(instances.data?.[0].node_name, 'synthetic-core-primary')
    assert.equal(instances.data?.[0].status, 'online')
    assert.equal(instances.data?.[1].status, 'stale')
    assert.equal(instances.data?.[1].info?.resources, undefined)
    assert.equal(instances.retention_days, 0)
    const resources = await getNativeSystemResources()
    assert.equal(resources.schema_version, 1)
    assert.equal(resources.system.logical_cpus, 8)
    assert.equal(resources.processes['lmm-core'].pid, 3024)
    assert.equal(resources.processes['lmm-extensions'].pid, 4176)
    resources.system.logical_cpus = 999
    assert.equal((await getNativeSystemResources()).system.logical_cpus, 8)
    for (const path of paths) {
      for (const method of ['post', 'put', 'patch', 'delete']) {
        await assert.rejects(wrapped(config(path, method)), /unmocked/)
      }
      for (const url of [
        `${path}/unlisted`,
        `https://example.invalid${path}`,
        `http://credential@127.0.0.1:4174${path}`,
      ]) {
        await assert.rejects(wrapped(config(url)), /unmocked/)
      }
    }
    await assert.rejects(
      wrapped(config('/core/v1/admin/resources')),
      /unmocked/
    )
    useAuthStore
      .getState()
      .auth.setUser({ id: 9002, username: 'review', role: 1 })
    for (const path of paths) {
      await assert.rejects(wrapped(config(path)), /unmocked/)
    }
  } finally {
    api.defaults.adapter = previousAdapter
    useAuthStore.getState().auth.setUser(previousUser)
  }
})

test('native wallet review serves current persona reads through the real API parsers', async () => {
  const { nativeWalletReviewProfile } =
    await import('./native-wallet-review-fixtures')
  const { commerceScope } = await import('@/features/commerce/native-client')
  const { getPublicPlans, getSelfSubscriptionFull } =
    await import('@/features/subscriptions/api')
  const { listNativePlans, listNativePurchases } =
    await import('@/features/subscriptions/native-api')
  const { listCampaigns, rewardHistory } =
    await import('@/features/promotions/native-api')
  const { getNativePaymentChannels } =
    await import('@/features/wallet/native-checkout-api')
  const { getTransferBalance, listTransfers } =
    await import('@/features/wallet/transfers/api')
  const { api } = await import('@/lib/api')
  const previousAuth = useAuthStore.getState().auth
  const previousAdapter = api.defaults.adapter
  const wrapped = withConsolePageFixtures(async () => {
    throw new Error('unmocked')
  })
  api.defaults.adapter = wrapped
  const reads = [
    '/api/wallet/balance',
    '/api/wallet-transfer',
    '/api/user/payments/channels',
    '/core/v1/subscriptions/plans',
    '/core/v1/subscriptions/purchases',
    '/core/v1/promotions/campaigns',
    '/core/v1/promotions/history',
  ]
  try {
    for (const persona of [
      { id: 1004, role: 1, name: 'e', balance: 750000 },
      { id: 1098, role: 10, name: 'manager', balance: 5000000 },
      { id: 1099, role: 100, name: 'admin', balance: 10000000 },
    ]) {
      const user = nativeWalletReviewProfile({
        id: persona.id,
        role: persona.role,
        username: `synthetic-${persona.name}`,
      })
      const sessionId = `debug-persona-${persona.name}`
      const expiresAt = Math.floor(Date.now() / 1000) + 86400
      useAuthStore.getState().auth.setBundle({
        access_token: sessionId,
        token_type: 'Bearer',
        access_expires_at: expiresAt,
        user,
        session: {
          sid: sessionId,
          current: true,
          login_method: 'development fixture',
          ip: '127.0.0.1',
          user_agent: 'synthetic fixture test',
          created_at: 1,
          last_active_at: 1,
          expires_at: expiresAt,
        },
      })
      const scope = commerceScope()
      assert.ok(scope)
      assert.equal(user.financial_projection_available, true)
      assert.equal(user.wallet_unit, 'core_credit')
      assert.equal(user.wallet_money_available, false)
      assert.equal(user.quota, persona.balance)
      const balance = await getTransferBalance(scope)
      assert.equal(balance.spendable_credits, user.wallet_spendable_quota)
      assert.equal(balance.public_credits_per_usd_exact, '500000')
      const transfers = await listTransfers(scope)
      assert.equal(transfers.length, 3)
      assert.equal(transfers[0].status, 'pending')
      assert.equal(transfers[0].quota, user.wallet_reserved_quota)
      assert.deepEqual(
        (await listTransfers(scope, transfers[0].id)).map((row) => row.id),
        transfers.slice(1).map((row) => row.id)
      )
      assert.equal((await listNativePlans(scope)).length, 2)
      assert.equal((await listNativePurchases(scope)).length, 1)
      assert.equal((await getPublicPlans(undefined, 'en')).data?.length, 2)
      assert.equal(
        (await getSelfSubscriptionFull()).data?.subscriptions.length,
        1
      )
      assert.equal((await listCampaigns(scope)).length, 2)
      assert.equal((await rewardHistory(scope))[0].amount_units, '10000')
      const channels = await getNativePaymentChannels()
      assert.equal(channels.available, true)
      assert.equal(channels.channels[0].environment, 'test')
      for (const provider of ['epay', 'pancake', 'stripe', 'creem']) {
        assert.deepEqual(
          (
            await wrapped(
              config(`/core/v1/subscriptions/plans/401/${provider}-prices`)
            )
          ).data,
          { prices: [] }
        )
      }
      for (const path of reads) {
        for (const method of ['post', 'put', 'patch', 'delete']) {
          await assert.rejects(wrapped(config(path, method)), /unmocked/)
        }
        for (const url of [
          `${path}/unlisted`,
          `https://example.invalid${path}`,
          `http://credential@127.0.0.1:4174${path}`,
        ]) {
          await assert.rejects(wrapped(config(url)), /unmocked/)
        }
        await assert.rejects(
          wrapped({
            ...config(path),
            authScope: { userId: persona.id + 1, sessionId },
          }),
          /unmocked/
        )
      }
      for (const url of [
        '/api/wallet-transfer?before=-1',
        '/api/wallet-transfer?before=1&before=2',
        '/api/wallet-transfer?before=9007199254740992',
      ]) {
        await assert.rejects(wrapped(config(url)), /unmocked/)
      }
      for (const path of [
        '/core/v1/subscriptions/quotes',
        '/core/v1/promotions/claims',
        '/api/user/payments/quote',
        '/api/user/payments/checkout',
        '/api/wallet-transfer/1/cancel',
      ]) {
        await assert.rejects(wrapped(config(path, 'post')), /unmocked/)
      }
    }
    useAuthStore.getState().auth.setUser(null)
    for (const path of reads) {
      await assert.rejects(wrapped(config(path)), /unmocked/)
    }
  } finally {
    api.defaults.adapter = previousAdapter
    useAuthStore.setState({ auth: previousAuth })
  }
})
