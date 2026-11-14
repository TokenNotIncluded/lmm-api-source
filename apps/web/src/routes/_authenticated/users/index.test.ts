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
import { describe, test } from 'node:test'

import {
  defaultParseSearch,
  defaultStringifySearch,
} from '@tanstack/react-router'

import {
  DEFAULT_NATIVE_USERS_QUERY,
  NATIVE_USERS_CAPABILITIES,
} from '@/features/native-users/contract'
import { nativeQueryFromSearch } from '@/features/native-users/search'

import { Route, usersSearchSchema } from './index'

describe('native user management search state', () => {
  test('route and page share the complete native query through router URL round trips', () => {
    const state = {
      keyword: '9007199254740993',
      group: ' exact ',
      role: 'admin',
      status: 'disabled',
      service_level: '0',
      page: 2,
      page_size: 37,
      sort_by: 'display_name',
      sort_order: 'asc',
    }
    assert.equal(Route.options.validateSearch, usersSearchSchema)
    const url = defaultStringifySearch(state)
    assert.deepEqual(usersSearchSchema.parse(defaultParseSearch(url)), state)
    assert.deepEqual(nativeQueryFromSearch(url), state)
    assert.deepEqual(usersSearchSchema.parse({}), DEFAULT_NATIVE_USERS_QUERY)
    assert.equal(
      usersSearchSchema.parse(defaultParseSearch('?service_level=0'))
        .service_level,
      '0'
    )
  })
  test('all native sorts are accepted while old or invalid filters fail instead of defaulting', () => {
    for (const sort_by of NATIVE_USERS_CAPABILITIES.sorts) {
      assert.equal(usersSearchSchema.parse({ sort_by }).sort_by, sort_by)
    }
    for (const state of [
      { pageSize: 20 },
      { filter: 'alice' },
      { role: ['10'] },
      { role: 10 },
      { status: ['2'] },
      { l0Only: false },
      { risk: 'all' },
      { sortBy: 'id' },
      { sort_by: 'quota' },
      { sort_order: 'up' },
      { service_level: 5 },
      { page: 0 },
      { page_size: 101 },
      { keyword: true },
      { group: '\u0000' },
      defaultParseSearch('?page=1&page=2'),
      defaultParseSearch('?role=user&role=admin'),
    ]) {
      assert.equal(
        usersSearchSchema.safeParse(state).success,
        false,
        JSON.stringify(state)
      )
    }
  })
})
