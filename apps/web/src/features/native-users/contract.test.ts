/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
// @ts-expect-error Bun's test module is only available in the test runtime.
import { expect, test } from 'bun:test'

import {
  DEFAULT_NATIVE_USERS_QUERY,
  NATIVE_USERS_CAPABILITIES,
  nativeGroupsResponseSchema,
  nativeUserSchema,
  nativeUserDetailSchema,
  nativeUserUsageSchema,
  nativeUsersQuery,
  nativeUsersResponseSchema,
  nativeWalletSchema,
} from './contract'
import { nativeQueryFromSearch } from './search'
import {
  nativeListFixture,
  nativeUserFixture,
  nativeUsageFixture,
} from './test-fixtures'

test('native identity and financial units remain exact strings beyond JS-safe IDs', () => {
  const parsed = nativeUsersResponseSchema.parse(nativeListFixture())
  expect(parsed.items[0].id).toBe('9007199254740993')
  expect(parsed.items[0].personal_account_id).toBe('9007199254740995')
  expect(parsed.items[0].auth_version).toBe('9007199254740997')
  expect(parsed.items[0].wallet).toMatchObject({
    available_units: '-7',
    wallet_units: '0',
    reserved_units: '3',
    debt_units: '7',
    currency: null,
    money_available: false,
  })
  const maximum = nativeUserFixture()
  if (maximum.wallet.status !== 'available') {
    throw new Error('synthetic fixture contract')
  }
  maximum.wallet.available_units = '-9007199254740991'
  maximum.wallet.debt_units = '9007199254740991'
  expect(nativeUserSchema.safeParse(maximum).success).toBe(true)
})

test('malformed, fractional, unsafe or negative-magnitude money is a validation failure, never an exception or zero', () => {
  for (const value of [
    'NaN',
    '1e3',
    '1.0',
    '-0',
    '+1',
    '01',
    '9007199254740992',
    '',
    '1'.repeat(1000),
  ]) {
    const wallet = nativeUserFixture().wallet
    if (wallet.status !== 'available') {
      throw new Error('synthetic fixture contract')
    }
    wallet.available_units = value
    expect(() => nativeWalletSchema.safeParse(wallet)).not.toThrow()
    expect(nativeWalletSchema.safeParse(wallet).success).toBe(false)
  }
  const wallet = nativeUserFixture().wallet
  expect(
    nativeWalletSchema.safeParse({ ...wallet, debt_units: '-7' }).success
  ).toBe(false)
  expect(nativeWalletSchema.safeParse({ ...wallet, unit: 'USD' }).success).toBe(
    false
  )
})

test('wallet provenance, activity, roles and profile availability are bound to the same row', () => {
  const row = nativeUserFixture()
  expect(nativeUserSchema.safeParse({ ...row, role: 10 }).success).toBe(false)
  expect(nativeUserSchema.safeParse({ ...row, status: 2 }).success).toBe(false)
  expect(
    nativeUserSchema.safeParse({ ...row, active: false, status: 2 }).success
  ).toBe(false)
  expect(
    nativeUserSchema.safeParse({ ...row, account_active: false }).success
  ).toBe(false)
  expect(
    nativeUserSchema.safeParse({ ...row, profile_available: false }).success
  ).toBe(false)
  expect(
    nativeUserSchema.safeParse({ ...row, id: '9007199254740994' }).success
  ).toBe(false)
  expect(
    nativeUserSchema.safeParse({
      ...row,
      active: false,
      status: 2,
      wallet: { status: 'unavailable', code: 'WALLET_SUBJECT_UNAVAILABLE' },
    }).success
  ).toBe(true)
  expect(
    nativeUserSchema.safeParse({
      ...row,
      profile_available: false,
      username: null,
      display_name: null,
      email: null,
    }).success
  ).toBe(true)
})

test('zero, negative and extreme safe timestamps are retained, null remains unknown', () => {
  for (const value of [
    0,
    -1,
    Number.MIN_SAFE_INTEGER,
    Number.MAX_SAFE_INTEGER,
    null,
  ]) {
    const row = nativeUserSchema.parse(
      nativeUserFixture({ registered_at_unix_ms: value })
    )
    expect(row.registered_at_unix_ms).toBe(value)
  }
  expect(
    nativeUserSchema.safeParse(
      nativeUserFixture({
        core_account_created_at_unix_ms: Number.MAX_SAFE_INTEGER + 1,
      })
    ).success
  ).toBe(false)
})

test('groups use exact unique UTF-8 C order and valid Unicode, not JS surrogate order', () => {
  expect(
    nativeUserSchema.safeParse(
      nativeUserFixture({ groups: ['\ue000', '\u{10000}'] })
    ).success
  ).toBe(true)
  for (const groups of [
    ['\u{10000}', '\ue000'],
    ['alpha', 'alpha'],
    [''],
    ['\ud800'],
    ['bad\u0085'],
  ]) {
    expect(
      nativeUserSchema.safeParse(nativeUserFixture({ groups })).success
    ).toBe(false)
  }
  expect(
    nativeUserSchema.parse(nativeUserFixture({ groups: [] })).groups
  ).toEqual([])
})

test('the complete capability and paging envelope cannot omit, invent or round fields', () => {
  const good = nativeListFixture()
  expect(nativeUsersResponseSchema.safeParse(good).success).toBe(true)
  const corrupt = [
    { ...good, total: Number.MAX_SAFE_INTEGER + 1 },
    { ...good, page: true },
    { ...good, items: [{ ...good.items[0], id: 9007199254740992 }] },
    { ...good, items: [...good.items, ...good.items], total: 2 },
    { ...good, page: 2 },
    {
      ...good,
      capabilities: {
        ...good.capabilities,
        implemented_write_actions: ['normal_ban'],
      },
    },
    { ...good, capabilities: { ...good.capabilities, filters: ['keyword'] } },
    { ...good, capabilities: undefined },
  ]
  for (const value of corrupt) {
    expect(nativeUsersResponseSchema.safeParse(value).success).toBe(false)
  }
  expect(
    nativeGroupsResponseSchema.safeParse({
      version: 1,
      items: ['alpha'],
      total: 21,
      page: 2,
      page_size: 20,
      source: 'enabled_explicit_user_routing_grants',
    }).success
  ).toBe(true)
  expect(
    nativeGroupsResponseSchema.safeParse({
      version: 1,
      items: ['alpha'],
      total: 1,
      page: 2,
      page_size: 20,
      source: 'enabled_explicit_user_routing_grants',
    }).success
  ).toBe(false)
})

test('supported queries retain literal metacharacters, exact groups and native level zero', () => {
  expect(
    nativeUsersQuery({
      ...DEFAULT_NATIVE_USERS_QUERY,
      keyword: '  %_  ',
      group: ' spaced ',
      service_level: '0',
    })
  ).toMatchObject({ keyword: '%_', group: ' spaced ', service_level: '0' })
  const invalid: unknown[] = [
    null,
    [],
    {},
    { ...DEFAULT_NATIVE_USERS_QUERY, l0Only: true },
    { ...DEFAULT_NATIVE_USERS_QUERY, sort_by: 'quota' },
    { ...DEFAULT_NATIVE_USERS_QUERY, keyword: '\ud800' },
    { ...DEFAULT_NATIVE_USERS_QUERY, group: '\ud800' },
    { ...DEFAULT_NATIVE_USERS_QUERY, keyword: '界'.repeat(86) },
    { ...DEFAULT_NATIVE_USERS_QUERY, page: 100001 },
  ]
  for (const value of invalid) {
    expect(() =>
      nativeUsersQuery(value as typeof DEFAULT_NATIVE_USERS_QUERY)
    ).toThrow('invalid_query')
  }
})

test('native URLs preserve all filters, exact text and every native sort', () => {
  expect(nativeQueryFromSearch('')).toEqual(DEFAULT_NATIVE_USERS_QUERY)
  const params = new URLSearchParams({
    page: '2',
    page_size: '50',
    keyword: JSON.stringify('9007199254740993'),
    group: ' spaced ',
    role: 'admin',
    status: 'disabled',
    service_level: '0',
    sort_by: 'display_name',
    sort_order: 'asc',
  })
  expect(nativeQueryFromSearch(params.toString())).toEqual({
    ...DEFAULT_NATIVE_USERS_QUERY,
    page: 2,
    page_size: 50,
    keyword: '9007199254740993',
    group: ' spaced ',
    role: 'admin',
    status: 'disabled',
    service_level: '0',
    sort_by: 'display_name',
    sort_order: 'asc',
  })
  for (const text of ['null', 'true', '123', '%_']) {
    expect(
      nativeQueryFromSearch(
        new URLSearchParams({ keyword: JSON.stringify(text) }).toString()
      )?.keyword
    ).toBe(text)
  }
  for (const sort of NATIVE_USERS_CAPABILITIES.sorts) {
    expect(nativeQueryFromSearch(`?sort_by=${sort}`)?.sort_by).toBe(sort)
  }
  for (const level of ['0', '1', '2', '3', '4']) {
    expect(
      nativeQueryFromSearch(`?service_level=${level}`)?.service_level
    ).toBe(level)
  }
})

test('old fields, invalid native filters and duplicate keys are rejected without dropping filters', () => {
  for (const search of [
    '?pageSize=20',
    '?filter=name',
    '?sortBy=id',
    '?sortOrder=asc',
    '?l0Only=false',
    '?risk=all',
    '?usage=all',
    '?funding=all',
    '?transfers=all',
    '?checkin=all',
    '?status=["2"]',
    '?role=["10"]',
    '?role=10',
    '?role=unknown',
    '?status=2',
    '?sort_by=quota',
    '?sort_by=unknown',
    '?service_level=5',
    '?service_level=true',
    '?page=1&page=2',
    '?role=user&role=admin',
    '?keyword=a&keyword=a',
    '?page_size=20&page_size=20',
    '?service_level=0&service_level=1',
    '?keyword=true',
    '?keyword=9007199254740993',
    '?page=0',
    '?page_size=101',
    '?page=1.5',
    '?page=100001',
    '?group=%00',
    '?sort_order=up',
  ]) {
    expect(nativeQueryFromSearch(search)).toBeUndefined()
  }
})

test('native usage preserves exact totals beyond safe integers and keeps unavailable distinct from zero', () => {
  const usage = nativeUsageFixture({
    settled_requests: '9007199254740993',
    gross_settled_units: '9007199254741193',
    net_consumption_units: '9007199254740993',
    personal_wallet_consumption_units: '9007199254740693',
  })
  const detail = nativeUserDetailSchema.parse({
    version: 1,
    user: nativeUserFixture(),
    usage,
  })
  expect(detail.usage).toEqual(usage)
  expect(
    nativeUserUsageSchema.parse({
      status: 'unavailable',
      code: 'USAGE_PROJECTION_UNAVAILABLE',
    })
  ).toEqual({
    status: 'unavailable',
    code: 'USAGE_PROJECTION_UNAVAILABLE',
  })
  expect(
    nativeUserDetailSchema.safeParse({
      version: 1,
      user: nativeUserFixture(),
    }).success
  ).toBe(false)
})

test('native usage rejects malformed totals and inconsistent refund or funding arithmetic', () => {
  for (const value of ['01', '-1', '1.5', '1e3', '', '1'.repeat(40), 1000]) {
    const usage = { ...nativeUsageFixture(), net_consumption_units: value }
    expect(() => nativeUserUsageSchema.safeParse(usage)).not.toThrow()
    expect(nativeUserUsageSchema.safeParse(usage).success).toBe(false)
  }
  for (const update of [
    { settled_requests: '12.5' },
    { pending_requests: -1 },
    { net_consumption_units: '1001' },
    { refunded_units: '1201' },
    { personal_wallet_consumption_units: '900' },
    { historical_imports_included: true },
    { scope: 'all_users' },
    { unit: 'USD' },
  ]) {
    expect(
      nativeUserUsageSchema.safeParse({
        ...nativeUsageFixture(),
        ...update,
      }).success
    ).toBe(false)
  }
})
