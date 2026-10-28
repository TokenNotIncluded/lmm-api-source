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
export const SUPPORT_TICKET_CATEGORIES = [
  'bounty_dispute',
  'refund',
  'invoice',
  'technical',
  'billing',
  'account',
  'other',
] as const

export const BOUNTY_DISPUTE_REASONS = [
  'merged_but_unpaid',
  'requirements_met_but_rejected',
  'misleading_requirements',
  'abusive_conduct',
  'other',
] as const

export type BountyDisputeReason = (typeof BOUNTY_DISPUTE_REASONS)[number]

export type SupportTicketCategory = (typeof SUPPORT_TICKET_CATEGORIES)[number]

export interface SupportTicketDraft {
  category: SupportTicketCategory
  categoryLabel: string
  contactEmail: string
  referenceId?: string
  subject: string
  details: string
  disputeReason?: BountyDisputeReason
}

export interface SupportTicketAccount {
  id?: number
  username?: string
}

export interface SupportTicketLabels {
  ticketType: string
  accountId: string
  username: string
  contactEmail: string
  referenceId: string
  subject: string
  details: string
}
