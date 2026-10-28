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

import type { AssistantStatus } from './api'
import { getAssistantAccountAccessState } from './assistant-access'

function status(developerAccessGranted: boolean): AssistantStatus {
  return {
    enabled: true,
    model: 'deepseek-v4-flash',
    developer_access_granted: developerAccessGranted,
    funding: {
      mode: 'super_administrator',
    },
  }
}

describe('assistant account access state', () => {
  test('restricts only an explicit L0 response', () => {
    assert.equal(
      getAssistantAccountAccessState(status(false), false),
      'restricted'
    )
    assert.equal(getAssistantAccountAccessState(status(true), false), 'granted')
  })

  test('keeps loading and failed status requests distinct from L0', () => {
    assert.equal(getAssistantAccountAccessState(undefined, false), 'loading')
    assert.equal(getAssistantAccountAccessState(undefined, true), 'error')
  })
})
