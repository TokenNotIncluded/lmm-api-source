/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { test } from 'node:test'

import { api } from '@/lib/api'

import { sendAssistantMessage } from './api'
import {
  INVITATION_PENDING,
  pendingInvitationError,
} from './assistant-invitation-contract'

function envelope() {
  return {
    success: false,
    code: INVITATION_PENDING,
    data: {
      conversation_id: 42,
      client_turn_id: 'original-turn',
      state: 'tools_pending',
    },
    lmm_assistant_invitation: {
      type: 'invitation_confirmation',
      tool: 'send_invitation',
      confirmation_id: 'a'.repeat(64),
      operation_id: 'original-invitation-operation',
      actor_user_id: 7,
      actor_credential_sha256: 'b'.repeat(64),
      expires_at: Math.floor(Date.now() / 1000) + 120,
      recipient: 'guest@example.invalid',
    },
  }
}
for (const mode of ['http202', 'sse', 'json'] as const) {
  test(`actual ${mode} client preserves the private pending envelope without resuming tools`, async () => {
    const originalFetch = globalThis.fetch,
      originalPost = api.post
    let calls = 0
    globalThis.fetch = async () => {
      calls++
      if (mode === 'json') throw new Error('No continuation request permitted')
      if (mode === 'sse') {
        return new Response(
          `event: error\ndata: ${JSON.stringify({
            ...envelope(),
            status: 202,
            retryable: false,
            message: 'Pending',
          })}\n\n`,
          { headers: { 'Content-Type': 'text/event-stream' } }
        )
      }
      return Response.json(envelope(), { status: 202 })
    }
    api.post = (async () => {
      calls++
      return { data: envelope(), status: 202, headers: {} }
    }) as typeof api.post
    try {
      await assert.rejects(
        sendAssistantMessage(
          'Send one invitation',
          [],
          undefined,
          undefined,
          mode === 'json' ? undefined : { onDelta: () => undefined },
          undefined,
          false,
          'original-turn'
        ),
        (error: unknown) =>
          !!pendingInvitationError(error, {
            client_turn_id: 'original-turn',
            conversation_id: 42,
            actor_user_id: 7,
          })
      )
      assert.equal(calls, 1)
    } finally {
      globalThis.fetch = originalFetch
      api.post = originalPost
    }
  })
}
