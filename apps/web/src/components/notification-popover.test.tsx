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

import type { TFunction } from 'i18next'

import {
  bountyNotificationPresentation,
  bountyNotificationSearch,
} from '@/features/open-source-bounties/notification-target'
import type {
  BountyNotification,
  BountyNotificationKind,
} from '@/features/open-source-bounties/types'

const t = ((key: string) => key) as TFunction

function notification(kind: BountyNotificationKind): BountyNotification {
  return {
    id: 1,
    project_id: 41,
    challenge_id: 72,
    sender_user_id: 2,
    sender_username: 'publisher',
    kind,
    project_title: 'Focused fix',
    quota: 2_000,
    note: 'tip note',
    recipient_read_at: 0,
    thanked_at: 0,
    created_at: 1,
  }
}

describe('bounty notification presentation', () => {
  test('distinguishes all transfer kinds and keeps note/thank tip-only', () => {
    const tip = bountyNotificationPresentation(notification('tip_transfer'), t)
    const reward = bountyNotificationPresentation(
      notification('reward_transfer'),
      t
    )
    const disputeReward = bountyNotificationPresentation(
      notification('dispute_reward_transfer'),
      t
    )

    assert.deepEqual(
      [tip.message, reward.message, disputeReward.message],
      [
        'sent you a tip of',
        'approved and paid your bounty reward',
        'paid your bounty reward after dispute resolution',
      ]
    )
    assert.deepEqual(
      [
        tip.showNoteAndThank,
        reward.showNoteAndThank,
        disputeReward.showNoteAndThank,
      ],
      [true, false, false]
    )
    assert.notEqual(tip.icon, reward.icon)
    assert.notEqual(reward.icon, disputeReward.icon)
  })

  test('builds the exact project and challenge search target', () => {
    assert.deepEqual(
      bountyNotificationSearch(notification('reward_transfer')),
      {
        projectId: 41,
        challengeId: 72,
      }
    )
  })
})
