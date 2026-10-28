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
import type { BountyChallenge } from './types'

export type ChallengeAcceptanceState =
  | 'available'
  | 'active'
  | 'completed'
  | 'retryable'

export function getChallengeAcceptanceState(
  challenge: BountyChallenge | null | undefined
): ChallengeAcceptanceState {
  if (!challenge) return 'available'

  switch (challenge.status) {
    case 'accepted':
    case 'submitted':
      return 'active'
    case 'approved':
      return 'completed'
    case 'rejected':
    case 'withdrawn':
    case 'cancelled':
      return 'retryable'
  }
}
