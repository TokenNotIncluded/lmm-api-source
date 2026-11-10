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

import {
  AssistantTurnContinuationError,
  AssistantTurnReleasedError,
  AssistantSubmissionRejectedError,
  AssistantTurnTerminalError,
  lookupAssistantTurn,
  recoverAssistantConversation,
  observeAssistantTurn,
  sendAssistantMessage,
} from './api'

const turn = { conversation_id: 42, client_turn_id: 'turn-1' }
const done = {
  choices: [{ message: { content: 'Settled answer' } }],
  lmm_assistant_history: turn,
}

for (const state of ['unknown', 'released', 'settled']) {
  test(`cancel ${state} uses an empty command and preserves server outcome`, async () => {
    const original = globalThis.fetch
    const calls: Array<{ url: string; init?: RequestInit }> = []
    globalThis.fetch = async (input, init) => {
      calls.push({ url: String(input), init })
      return state === 'settled'
        ? Response.json(done)
        : Response.json(
            {
              success: false,
              code:
                state === 'released'
                  ? 'ASSISTANT_OPERATION_RELEASED'
                  : 'ASSISTANT_OPERATION_PENDING',
              data: { ...turn, state },
            },
            { status: state === 'released' ? 409 : 202 }
          )
    }
    try {
      const result = observeAssistantTurn(turn, {}, undefined, true)
      if (state === 'settled') {
        assert.equal((await result).content, 'Settled answer')
      } else {
        await assert.rejects(
          result,
          state === 'released'
            ? AssistantTurnReleasedError
            : AssistantTurnContinuationError
        )
      }
      assert.equal(calls.length, 1)
      assert.equal(
        calls[0]?.url,
        '/api/assistant/conversations/42/turns/turn-1/cancel'
      )
      assert.equal(calls[0]?.init?.method, 'POST')
      assert.equal(calls[0]?.init?.body, '{}')
    } finally {
      globalThis.fetch = original
    }
  })
}

test('output observes a snapshot without a cursor, body or resume command', async () => {
  const original = globalThis.fetch
  globalThis.fetch = async (input, init) => {
    assert.equal(
      String(input),
      '/api/assistant/conversations/42/turns/turn-1/output'
    )
    assert.equal(init?.method, 'GET')
    assert.equal(init?.body, undefined)
    const headers = new Headers(init?.headers)
    assert.equal(headers.get('Accept'), 'text/event-stream')
    assert.equal(headers.has('Last-Event-ID'), false)
    return new Response(`event: done\ndata: ${JSON.stringify(done)}\n\n`, {
      headers: { 'content-type': 'text/event-stream' },
    })
  }
  try {
    assert.equal(
      (await observeAssistantTurn(turn, {})).content,
      'Settled answer'
    )
  } finally {
    globalThis.fetch = original
  }
})

test('disconnected output never replays chat POST', async () => {
  const original = globalThis.fetch
  let calls = 0
  globalThis.fetch = async () => {
    calls += 1
    return new Response(
      `event: ready\ndata: ${JSON.stringify(turn)}\n\nevent: replace\ndata: {"content":"Tentative"}\n\n`,
      { headers: { 'content-type': 'text/event-stream' } }
    )
  }
  try {
    await assert.rejects(
      sendAssistantMessage(
        'Hello',
        [],
        42,
        undefined,
        { onDelta: () => undefined },
        undefined,
        false,
        turn.client_turn_id
      ),
      /before completion/
    )
    assert.equal(calls, 1)
  } finally {
    globalThis.fetch = original
  }
})

for (const cancel of [false, true]) {
  test(`rejects a different turn on ${cancel ? 'cancel' : 'output'} JSON fallback`, async () => {
    const original = globalThis.fetch
    globalThis.fetch = async () =>
      Response.json({
        ...done,
        lmm_assistant_history: { ...turn, conversation_id: 99 },
      })
    try {
      await assert.rejects(
        observeAssistantTurn(turn, {}, undefined, cancel),
        /different turn/
      )
    } finally {
      globalThis.fetch = original
    }
  })
}

for (const status of [400, 401, 403]) {
  test(`initial HTTP ${status} is a refused submission, not an unknown turn`, async () => {
    const original = globalThis.fetch
    let calls = 0
    globalThis.fetch = async () => {
      calls += 1
      return Response.json(
        { success: false, message: 'Rejected', code: 'ASSISTANT_FORBIDDEN' },
        { status }
      )
    }
    try {
      await assert.rejects(
        sendAssistantMessage(
          'Hello',
          [],
          undefined,
          undefined,
          { onDelta: () => undefined },
          undefined,
          false,
          turn.client_turn_id
        ),
        (error) =>
          error instanceof AssistantSubmissionRejectedError &&
          error.status === status
      )
      assert.equal(calls, 1)
    } finally {
      globalThis.fetch = original
    }
  })
}

for (const phase of ['registered', 'creation_inflight']) {
  test(`lookup ${phase} validates the original identity with a bodyless GET`, async () => {
    const original = globalThis.fetch
    globalThis.fetch = async (input, init) => {
      assert.equal(String(input), '/api/assistant/turns/turn-1/conversation')
      assert.equal(init?.method, 'GET')
      assert.equal(init?.body, undefined)
      assert.equal(new Headers(init?.headers).has('Last-Event-ID'), false)
      return Response.json(
        { success: true, message: '', data: { ...turn, phase } },
        { status: phase === 'registered' ? 200 : 202 }
      )
    }
    try {
      const mapping = await lookupAssistantTurn(turn.client_turn_id)
      assert.equal(mapping.conversation_id, 42)
      assert.equal(mapping.client_turn_id, turn.client_turn_id)
      assert.equal(mapping.phase, phase)
    } finally {
      globalThis.fetch = original
    }
  })
}

for (const status of [404, 401, 403, 503]) {
  test(`lookup HTTP ${status} cannot authorize replay or claim cancellation`, async () => {
    const original = globalThis.fetch
    const methods: Array<string | undefined> = []
    globalThis.fetch = async (_input, init) => {
      methods.push(init?.method)
      return Response.json(
        {
          success: false,
          message: 'Unavailable',
          code: 'ASSISTANT_HISTORY_NOT_FOUND',
        },
        { status }
      )
    }
    try {
      await assert.rejects(
        lookupAssistantTurn(turn.client_turn_id),
        (error) =>
          error instanceof Error &&
          !(error instanceof AssistantSubmissionRejectedError) &&
          !(error instanceof AssistantTurnReleasedError)
      )
      assert.deepEqual(methods, ['GET'])
    } finally {
      globalThis.fetch = original
    }
  })
}

for (const data of [
  { ...turn, client_turn_id: 'other-turn', phase: 'registered' },
  { ...turn, conversation_id: 0, phase: 'registered' },
  { ...turn, phase: 'creation_inflight' },
]) {
  test(`lookup rejects an invalid receipt ${JSON.stringify(data)}`, async () => {
    const original = globalThis.fetch
    globalThis.fetch = async () => Response.json({ success: true, data })
    try {
      await assert.rejects(lookupAssistantTurn(turn.client_turn_id))
    } finally {
      globalThis.fetch = original
    }
  })
}

for (const code of [
  'ASSISTANT_OPERATION_REJECTED',
  'ASSISTANT_OPERATION_EXPIRED',
]) {
  test(`original turn ${code} is terminal rather than permanently pending`, async () => {
    const original = globalThis.fetch
    globalThis.fetch = async () =>
      Response.json({ success: false, code, data: turn }, { status: 409 })
    try {
      await assert.rejects(
        observeAssistantTurn(turn, {}),
        AssistantTurnTerminalError
      )
    } finally {
      globalThis.fetch = original
    }
  })
}

for (const unknown of [false, true]) {
  test(`explicit receipt recovery ${unknown ? 'UNKNOWN' : 'registered'} never submits chat`, async () => {
    const original = globalThis.fetch
    globalThis.fetch = async (input, init) => {
      assert.equal(
        String(input),
        '/api/assistant/turns/turn-1/conversation/recover'
      )
      assert.equal(init?.method, 'POST')
      assert.equal(init?.body, '{}')
      const data = {
        ...turn,
        phase: unknown ? 'creation_inflight' : 'registered',
      }
      return unknown
        ? Response.json(
            {
              success: false,
              code: 'ASSISTANT_CONVERSATION_RECOVERY_UNKNOWN',
              data,
            },
            { status: 202 }
          )
        : Response.json({ success: true, data })
    }
    try {
      if (unknown) {
        await assert.rejects(
          recoverAssistantConversation(turn),
          (error) =>
            error instanceof AssistantTurnContinuationError &&
            error.code === 'ASSISTANT_CONVERSATION_RECOVERY_UNKNOWN'
        )
      } else {
        assert.equal(
          (await recoverAssistantConversation(turn)).phase,
          'registered'
        )
      }
    } finally {
      globalThis.fetch = original
    }
  })
}

test('receipt recovery cannot rebind the browser to another conversation', async () => {
  const original = globalThis.fetch
  globalThis.fetch = async () =>
    Response.json(
      {
        success: false,
        code: 'ASSISTANT_CONVERSATION_RECOVERY_UNKNOWN',
        data: { ...turn, conversation_id: 99, phase: 'creation_inflight' },
      },
      { status: 202 }
    )
  try {
    await assert.rejects(recoverAssistantConversation(turn), /different turn/)
  } finally {
    globalThis.fetch = original
  }
})

for (const mismatched of [false, true]) {
  test(`cancel-before-start fence ${mismatched ? 'rejects another identity' : 'is the only early cancellation receipt'}`, async () => {
    const original = globalThis.fetch
    globalThis.fetch = async () =>
      Response.json({
        success: true,
        data: {
          ...turn,
          conversation_id: mismatched ? 99 : 42,
          state: 'cancelled_before_start',
        },
      })
    try {
      if (mismatched) {
        await assert.rejects(
          observeAssistantTurn(turn, {}, undefined, true),
          /different turn/
        )
      } else {
        await assert.rejects(
          observeAssistantTurn(turn, {}, undefined, true),
          (error) =>
            error instanceof AssistantTurnReleasedError && error.beforeStart
        )
      }
    } finally {
      globalThis.fetch = original
    }
  })
}

test('an error event after ready is not a submission-time rejection', async () => {
  const original = globalThis.fetch
  globalThis.fetch = async () =>
    new Response(
      `event: ready\ndata: ${JSON.stringify(turn)}\n\nevent: error\ndata: {"status":403,"message":"Original turn interrupted","retryable":false}\n\n`,
      { headers: { 'content-type': 'text/event-stream' } }
    )
  try {
    await assert.rejects(
      sendAssistantMessage(
        'Hello',
        [],
        42,
        undefined,
        { onDelta: () => undefined },
        undefined,
        false,
        turn.client_turn_id
      ),
      (error) =>
        error instanceof Error &&
        !(error instanceof AssistantSubmissionRejectedError)
    )
  } finally {
    globalThis.fetch = original
  }
})
