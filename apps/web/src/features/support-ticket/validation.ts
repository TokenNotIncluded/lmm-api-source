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
import { z } from 'zod'

import { BOUNTY_DISPUTE_REASONS, SUPPORT_TICKET_CATEGORIES } from './types'

export const supportTicketSchema = z
  .object({
    category: z.enum(SUPPORT_TICKET_CATEGORIES),
    disputeReason: z.enum(BOUNTY_DISPUTE_REASONS),
    contactEmail: z.string().trim(),
    referenceId: z.string().trim().max(120, 'Reference ID is too long'),
    subject: z
      .string()
      .trim()
      .min(4, 'Subject must be at least 4 characters')
      .max(100, 'Subject must be at most 100 characters'),
    details: z
      .string()
      .trim()
      .min(20, 'Details must be at least 20 characters')
      .max(1200, 'Details must be at most 1200 characters'),
  })
  .superRefine((values, context) => {
    if (
      values.category !== 'bounty_dispute' &&
      !z.email().safeParse(values.contactEmail).success
    ) {
      context.addIssue({
        code: 'custom',
        path: ['contactEmail'],
        message: 'Enter a valid contact email',
      })
    }
    if (
      values.category === 'bounty_dispute' &&
      !/^\d+$/.test(values.referenceId)
    ) {
      context.addIssue({
        code: 'custom',
        path: ['referenceId'],
        message: 'Enter a valid bounty challenge ID',
      })
    }
  })

export type SupportTicketForm = z.infer<typeof supportTicketSchema>
