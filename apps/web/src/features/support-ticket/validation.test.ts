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

import { supportTicketSchema } from './validation'

const validTicket = {
  category: 'technical' as const,
  disputeReason: 'other' as const,
  contactEmail: 'contributor@example.com',
  referenceId: '',
  subject: 'Request review',
  details: 'The request contains enough factual detail for a review.',
}

describe('support ticket validation', () => {
  test('does not require an email for an authenticated bounty dispute', () => {
    const result = supportTicketSchema.safeParse({
      ...validTicket,
      category: 'bounty_dispute',
      contactEmail: '',
      referenceId: '42',
    })

    assert.equal(result.success, true)
  })

  test('still requires an email for email-based support requests', () => {
    const result = supportTicketSchema.safeParse({
      ...validTicket,
      contactEmail: '',
    })

    assert.equal(result.success, false)
  })
})
