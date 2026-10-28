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

import { buildSupportMailto, buildSupportTicketText } from './lib'

const labels = {
  ticketType: 'Ticket type',
  accountId: 'Account ID',
  username: 'Username',
  contactEmail: 'Contact email',
  referenceId: 'Reference ID',
  subject: 'Subject',
  details: 'Details',
}

describe('support ticket mail composer', () => {
  test('includes the authenticated account and user-provided reference', () => {
    const text = buildSupportTicketText(
      {
        category: 'refund',
        categoryLabel: 'Refund request',
        contactEmail: 'user@example.com',
        referenceId: 'order-123',
        subject: 'Duplicate payment',
        details: 'The same order was charged twice.',
      },
      { id: 42, username: 'alice' },
      labels
    )

    assert.match(text, /Ticket type: Refund request/)
    assert.match(text, /Account ID: 42/)
    assert.match(text, /Username: alice/)
    assert.match(text, /Reference ID: order-123/)
    assert.match(text, /Details:\nThe same order was charged twice\./)
  })

  test('omits empty optional fields and safely encodes the mailto URL', () => {
    const text = buildSupportTicketText(
      {
        category: 'technical',
        categoryLabel: 'Technical support',
        contactEmail: 'user@example.com',
        subject: 'API returns 500',
        details: 'Endpoint: /v1/responses',
      },
      {},
      labels
    )
    const mailto = buildSupportMailto('[Support ticket] API returns 500', text)
    const url = new URL(mailto)

    assert.doesNotMatch(text, /Account ID:/)
    assert.doesNotMatch(text, /Reference ID:/)
    assert.equal(url.protocol, 'mailto:')
    assert.equal(url.pathname, 'support@lmm.best')
    assert.equal(
      url.searchParams.get('subject'),
      '[Support ticket] API returns 500'
    )
    assert.equal(url.searchParams.get('body'), text)
  })
})
