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

import { selectBountyNotificationChallenge } from './notification-target'
import type { BountyProjectDetail } from './types'

const challenge = (id: number): BountyProjectDetail['challenges'][number] => ({
  id,
  project_id: 41,
  participant_user_id: 2,
  github_handle: 'contributor',
  status: 'accepted',
  issue_url: '',
  pull_request_url: '',
  submission_note: '',
  review_note: '',
  reward_quota: 0,
  tip_quota: 0,
  owner_rating_score: 0,
  owner_rating_comment: '',
  owner_rated_at: 0,
  contributor_rating_score: 0,
  contributor_rating_comment: '',
  contributor_rated_at: 0,
  accepted_at: 0,
  submitted_at: 0,
  reviewed_at: 0,
  paid_at: 0,
})

const detail: BountyProjectDetail = {
  project: {
    id: 41,
    owner_user_id: 1,
    owner_username: 'owner',
    repository_url: 'https://github.com/example/project',
    title: 'Project',
    description: '',
    rules: '',
    reward_quota: 0,
    net_reward_quota: 0,
    reward_slots: 1,
    escrow_quota: 0,
    platform_fee_rate_bps: 0,
    platform_fee_quota: 0,
    status: 'published',
    created_at: 0,
    updated_at: 0,
    published_at: 0,
    closed_at: 0,
    archived_at: 0,
    active_challenge_count: 2,
    approved_challenge_count: 0,
    owner_rating_average: 0,
    owner_rating_count: 0,
    owner_thank_heart_count: 0,
  },
  challenges: [challenge(71), challenge(72)],
  ledger: [],
}

describe('bounty notification detail target', () => {
  test('selects the exact challenge within the requested project', () => {
    assert.equal(
      selectBountyNotificationChallenge(detail, {
        projectId: 41,
        challengeId: 72,
      })?.id,
      72
    )
  })

  test('rejects project and challenge identity mismatches', () => {
    assert.equal(
      selectBountyNotificationChallenge(detail, {
        projectId: 42,
        challengeId: 72,
      }),
      null
    )
    assert.equal(
      selectBountyNotificationChallenge(detail, {
        projectId: 41,
        challengeId: 99,
      }),
      null
    )
  })
})
