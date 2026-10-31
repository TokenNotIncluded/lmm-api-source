/*
Copyright (C) 2026 LIghtJUNction

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
*/
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { api } from '@/lib/api'

import { sendAssistantMessage } from './api'

test('the existing streaming entry point resumes JSON turns with an empty command', async () => {
  const calls: Array<{
    url: string
    method: string | undefined
    body: BodyInit | null | undefined
  }> = []
  const original = Object.getOwnPropertyDescriptor(globalThis, 'fetch')
  assert.ok(original)
  Object.defineProperty(globalThis, 'fetch', {
    ...original,
    value: async (input: RequestInfo | URL, init?: RequestInit) => {
      calls.push({ url: String(input), method: init?.method, body: init?.body })
      return calls.length === 1
        ? Response.json(
            {
              success: false,
              code: 'ASSISTANT_OPERATION_PENDING',
              data: {
                conversation_id: 42,
                client_turn_id: 'original-turn',
                state: 'tools_pending',
              },
            },
            { status: 202 }
          )
        : Response.json({
            success: true,
            choices: [{ message: { content: 'Original persisted answer' } }],
            lmm_assistant_history: {
              conversation_id: 42,
              client_turn_id: 'original-turn',
            },
          })
    },
  })
  try {
    const reply = await sendAssistantMessage(
      'Original message',
      [],
      undefined,
      undefined,
      { onDelta: () => undefined },
      undefined,
      false,
      'original-turn'
    )
    assert.equal(reply.content, 'Original persisted answer')
    assert.equal(reply.conversationId, 42)
    assert.equal(calls.length, 2)
    assert.equal(calls[0]?.url, '/api/assistant/chat')
    assert.deepEqual(calls[1], {
      url: '/api/assistant/conversations/42/turns/original-turn/resume',
      method: 'POST',
      body: '{}',
    })
  } finally {
    Object.defineProperty(globalThis, 'fetch', original)
  }
})

for (const transport of ['json', 'stream'] as const) {
  test(`${transport} starter submission preserves edits and only reads pending PersonalText turns`, async () => {
    const calls: Array<{ url: string; method: string; body?: unknown }> = []
    const originalFetch = Object.getOwnPropertyDescriptor(globalThis, 'fetch')
    const originalPost = api.post
    assert.ok(originalFetch)
    const states = ['reserved', 'streaming', 'unknown'] as const
    const respond = (url: string, method: string, body?: unknown) => {
      calls.push({ url, method, ...(body === undefined ? {} : { body }) })
      const state = states[calls.length - 1]
      return state
        ? Response.json(
            {
              success: false,
              code: 'ASSISTANT_OPERATION_PENDING',
              data: {
                conversation_id: 42,
                client_turn_id: 'starter-turn',
                state,
              },
            },
            { status: 202 }
          )
        : Response.json({
            success: true,
            choices: [
              { message: { content: 'Persisted edited-starter answer' } },
            ],
            lmm_assistant_history: {
              conversation_id: 42,
              client_turn_id: 'starter-turn',
            },
          })
    }
    Object.defineProperty(globalThis, 'fetch', {
      ...originalFetch,
      value: async (input: RequestInfo | URL, init?: RequestInit) =>
        respond(
          String(input),
          init?.method ?? 'GET',
          typeof init?.body === 'string' ? JSON.parse(init.body) : undefined
        ),
    })
    api.post = (async (url: string, body: unknown) => {
      const response = respond(url, 'POST', body)
      return {
        data: await response.json(),
        status: response.status,
        headers: {},
      }
    }) as typeof api.post
    try {
      const edited = 'Where should I start?\n请先解释团队访问。'
      const reply = await sendAssistantMessage(
        edited,
        [],
        undefined,
        'getting_started',
        transport === 'stream' ? { onDelta: () => undefined } : undefined,
        undefined,
        false,
        'starter-turn'
      )
      assert.equal(reply.content, 'Persisted edited-starter answer')
      assert.equal(reply.conversationId, 42)
      assert.deepEqual(calls, [
        {
          url: '/api/assistant/chat',
          method: 'POST',
          body: {
            message: edited,
            messages: [{ role: 'user', content: edited }],
            preset_id: 'getting_started',
            client_turn_id: 'starter-turn',
          },
        },
        ...Array.from({ length: 3 }, () => ({
          url: '/api/assistant/conversations/42/turns/starter-turn',
          method: 'GET',
        })),
      ])
    } finally {
      api.post = originalPost
      Object.defineProperty(globalThis, 'fetch', originalFetch)
    }
  })
}
