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

import { getChallengeAcceptanceState } from './acceptance'
import type { BountyChallenge, BountyChallengeStatus } from './types'

function challenge(status: BountyChallengeStatus): BountyChallenge {
  return { status } as BountyChallenge
}

describe('challenge acceptance state', () => {
  test('allows a first acceptance when there is no prior attempt', () => {
    assert.equal(getChallengeAcceptanceState(undefined), 'available')
  })

  test('keeps accepted and submitted attempts active', () => {
    assert.equal(getChallengeAcceptanceState(challenge('accepted')), 'active')
    assert.equal(getChallengeAcceptanceState(challenge('submitted')), 'active')
  })

  test('keeps an approved delivery terminal', () => {
    assert.equal(
      getChallengeAcceptanceState(challenge('approved')),
      'completed'
    )
  })

  test('offers a new attempt after every non-paying terminal state', () => {
    for (const status of ['rejected', 'withdrawn', 'cancelled'] as const) {
      assert.equal(getChallengeAcceptanceState(challenge(status)), 'retryable')
    }
  })
})
