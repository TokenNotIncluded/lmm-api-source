/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
// @ts-expect-error Bun's test module is only available in the test runtime.
import { expect, test } from 'bun:test'

import { nativeUsersCsv } from './export'
import { nativeUserFixture } from './test-fixtures'

test('CSV preserves exact user IDs and signed credit amounts as spreadsheet text', () => {
  const user = nativeUserFixture()
  if (user.wallet.status !== 'available') throw new Error('fixture wallet')
  user.wallet.available_units = '-9007199254740991'
  user.wallet.debt_units = '9007199254740991'
  user.registered_at_unix_ms = Number.MAX_SAFE_INTEGER
  const csv = nativeUsersCsv([user], ['User ID'])
  expect(csv.startsWith('\uFEFF')).toBe(true)
  expect(csv).toContain('"\'9007199254740993"')
  expect(csv).toContain('"\'-9007199254740991"')
  expect(csv).toContain('"\'9007199254740991"')
  expect(csv).toContain('"core_credit"')
  expect(csv).not.toContain('9007199254740992')
  expect(csv).not.toContain('USD')
})

test('CSV escapes line breaks and quotes, neutralizes formula-like user input and keeps group boundaries', () => {
  const user = nativeUserFixture({
    username: '\t=HYPERLINK("https://example.invalid")',
    display_name: '+SUM(1,1)',
    email: '-1+2',
    groups: ['comma,group', 'line"quote'],
  })
  const csv = nativeUsersCsv([user], ['User ID'])
  expect(csv).toContain('"\'\t=HYPERLINK(""https://example.invalid"")"')
  expect(csv).toContain('"\'+SUM(1,1)"')
  expect(csv).toContain('"\'-1+2"')
  expect(csv).toContain('"[""comma,group"",""line\\""quote""]"')
  const multiline = nativeUsersCsv(
    [nativeUserFixture({ display_name: 'one\ntwo' })],
    []
  )
  expect(multiline).toContain('"one\ntwo"')
})

test('an unavailable wallet exports no invented amount and null profiles keep their availability marker', () => {
  const csv = nativeUsersCsv(
    [
      nativeUserFixture({
        profile_available: false,
        username: null,
        display_name: null,
        email: null,
        registered_at_unix_ms: null,
        wallet: { status: 'unavailable', code: 'WALLET_SUBJECT_UNAVAILABLE' },
      }),
    ],
    []
  )
  expect(csv).toContain('"\'9007199254740993","false","","",""')
  expect(csv).toContain('"unavailable","","","","",""\r\n')
  expect(csv).not.toContain('core_credit')
})
