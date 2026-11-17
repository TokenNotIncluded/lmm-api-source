/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.
*/
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'

const source = readFileSync(new URL('./use-sidebar-data.ts', import.meta.url), 'utf8')
test('one console navigation serves every signed-in user', () => {
  assert.doesNotMatch(source, /id: 'onboarding'/)
  for (const path of ['/keys', '/guide', '/usage-logs/common', '/wallet']) {
    assert.ok(source.includes(`url: '${path}'`))
  }
  assert.doesNotMatch(source, /remote-control|Remote control/)
})
