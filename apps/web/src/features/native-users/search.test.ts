/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
// @ts-expect-error Bun's test module is only available in the test runtime.
import { expect, test } from 'bun:test'

import { DEFAULT_NATIVE_USERS_QUERY } from './contract'
import { nativeQueryFromSearch, nativeUsersSearch } from './search'

test('copied filter URLs preserve long numeric IDs, exact groups and the applied page', () => {
  const query = {
    ...DEFAULT_NATIVE_USERS_QUERY,
    keyword: '9007199254740993',
    group: ' exact ',
    service_level: '0' as const,
    role: 'admin' as const,
    status: 'disabled' as const,
    page: 2,
    page_size: 37,
    sort_by: 'groups' as const,
    sort_order: 'asc' as const,
  }
  expect(nativeQueryFromSearch(nativeUsersSearch(query))).toEqual(query)
  for (const group of ['true', 'null', '9007199254740997', 'a&b', '模型']) {
    expect(
      nativeQueryFromSearch(nativeUsersSearch({ ...query, group }))?.group
    ).toBe(group)
  }
})

test('default filters produce a clean user URL and invalid queries are not copied', () => {
  expect(nativeUsersSearch(DEFAULT_NATIVE_USERS_QUERY)).toBe('')
  expect(
    nativeQueryFromSearch(nativeUsersSearch(DEFAULT_NATIVE_USERS_QUERY))
  ).toEqual(DEFAULT_NATIVE_USERS_QUERY)
  expect(() =>
    nativeUsersSearch({ ...DEFAULT_NATIVE_USERS_QUERY, page: 0 })
  ).toThrow()
})
