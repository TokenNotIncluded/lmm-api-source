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
/*
Copyright (C) 2026 LIghtJUNction
*/
import assert from 'node:assert/strict'
import { describe, test } from 'node:test'

import type { TodoItem } from './api'
import { todoItemTitleKey } from './todo-labels'
import { todoItemCanOpen, todoItemHasDestination } from './todo-navigation'

function item(
  category: TodoItem['category'],
  details: Record<string, unknown> = {}
): TodoItem {
  return {
    id: `${category}:1`,
    source_id: 1,
    category,
    type: 'test',
    title: 'test',
    summary: 'test',
    read: false,
    created_at: 1,
    updated_at: 1,
    details,
  }
}

describe('unified todo destinations', () => {
  test('uses a safety incident label and falls back for unknown notifications', () => {
    assert.equal(
      todoItemTitleKey('assistant.security_incident'),
      'Assistant safety incident'
    )
    assert.equal(todoItemTitleKey('unknown.todo'), 'Notification')
    assert.equal(todoItemTitleKey('Low balance'), 'Low balance')
  })

  test('keeps destination affordances tied to actionable notification data', () => {
    assert.equal(todoItemHasDestination(item('open_source_bounty')), false)
    assert.equal(
      todoItemHasDestination(item('open_source_bounty', { project_id: 12 })),
      true
    )
    assert.equal(todoItemHasDestination(item('security_incident')), false)
    assert.equal(
      todoItemHasDestination(
        item('security_incident', { username: 'customer' })
      ),
      true
    )
    // A valid request ID keeps account-action and support queues actionable.
    assert.equal(todoItemHasDestination(item('account_action')), true)
    assert.equal(todoItemHasDestination(item('human_support')), true)
    assert.equal(
      todoItemTitleKey('assistant.human_support'),
      'Human technical support'
    )
  })
  test('lets an owner open only the typed low-balance account notification', () => {
    const notice = { ...item('account_action'), type: 'low_balance' }
    for (const isAdmin of [false, true]) {
      assert.equal(todoItemHasDestination(notice), true)
      assert.equal(todoItemCanOpen(notice, isAdmin), true)
    }
    assert.equal(todoItemCanOpen(item('account_action'), false), false)
    assert.equal(
      todoItemCanOpen(
        { ...notice, type: 'request', title: 'Low balance' },
        false
      ),
      false
    )
    assert.equal(
      todoItemCanOpen({ ...notice, category: 'security_incident' }, false),
      false
    )
  })
})
