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
import type {
  SupportTicketAccount,
  SupportTicketDraft,
  SupportTicketLabels,
} from './types'

export const SUPPORT_EMAIL = 'support@lmm.best'

export function buildSupportTicketText(
  draft: SupportTicketDraft,
  account: SupportTicketAccount,
  labels: SupportTicketLabels
): string {
  const lines = [
    `${labels.ticketType}: ${draft.categoryLabel}`,
    account.id ? `${labels.accountId}: ${account.id}` : '',
    account.username ? `${labels.username}: ${account.username}` : '',
    `${labels.contactEmail}: ${draft.contactEmail}`,
    draft.referenceId
      ? `${labels.referenceId}: ${draft.referenceId.trim()}`
      : '',
    `${labels.subject}: ${draft.subject.trim()}`,
    '',
    `${labels.details}:`,
    draft.details.trim(),
  ]

  return lines
    .filter((line, index) => line !== '' || index >= lines.length - 3)
    .join('\n')
}

export function buildSupportMailto(subject: string, body: string): string {
  const params = new URLSearchParams({ subject, body })
  return `mailto:${SUPPORT_EMAIL}?${params.toString()}`
}
