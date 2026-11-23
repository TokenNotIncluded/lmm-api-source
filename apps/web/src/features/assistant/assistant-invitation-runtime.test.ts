/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { test } from 'node:test'

import { AssistantStreamError } from './assistant-ai-stream'
import {
  INVITATION_PENDING,
  parsePendingInvitation,
  pendingInvitationError,
  parseInvitationView,
  parseInvitationOutcome,
  type InvitationAction,
} from './assistant-invitation-contract'
import {
  createInvitationRuntime,
  type InvitationOwner,
} from './assistant-invitation-runtime'
import { continueAssistantResponse } from './assistant-turn-recovery'

function fixture() {
  let time = 1_000_000
  let owner: InvitationOwner | undefined = {
    userID: 7,
    sessionID: 'original',
    accessToken: 'synthetic-session-original',
  }
  const action: InvitationAction = {
    type: 'invitation_confirmation',
    tool: 'send_invitation',
    confirmation_id: 'a'.repeat(64),
    operation_id: 'original-invitation-operation',
    actor_user_id: 7,
    actor_credential_sha256: createHash('sha256')
      .update(owner.accessToken)
      .digest('hex'),
    expires_at: 1120,
    recipient: 'guest@example.invalid',
  }
  const view = () => ({
    id: action.confirmation_id,
    operation_id: action.operation_id,
    tool: 'send_invitation',
    browser_url: `/api/user/auth/assistant/confirmations/${'b'.repeat(64)}`,
    payload: {
      kind: 'invitation_email_v1',
      email: action.recipient,
      invitation_code: 'Ab12',
    },
    expires_at: action.expires_at,
    consumed_at: null as number | null,
    human_confirmed: false,
    dispatch_started: false,
  })
  const outcome = () => ({
    ok: true,
    applied: true,
    status: 'accepted_by_smtp',
    smtp_accepted: true,
    smtp_code: 250,
    delivery_confirmed: false,
    retry_safe: false,
    operation_id: action.operation_id,
    confirmation_id: action.confirmation_id,
    proof_receipt_id: 'synthetic-proof',
  })
  const envelope = () => ({
    success: false,
    code: INVITATION_PENDING,
    data: {
      conversation_id: 42,
      client_turn_id: 'original',
      state: 'tools_pending',
    },
    lmm_assistant_invitation: { ...action },
  })
  let respond: (method: 'GET' | 'POST') => Promise<unknown> = async () => ({
    success: true,
    data: view(),
  })
  const requests: Array<{ method: string; path: string }> = []
  const attempts = new Set<string>()
  const make = () =>
    createInvitationRuntime(
      action,
      {
        owner: () => owner,
        request: async (method, path) => {
          requests.push({ method, path })
          return respond(method)
        },
      },
      () => time,
      attempts
    )
  return {
    action,
    view,
    outcome,
    envelope,
    requests,
    make,
    attempts,
    setResponse: (value: typeof respond) => {
      respond = value
    },
    switch: (value: InvitationOwner | undefined) => {
      owner = value
    },
    tick: (value: number) => {
      time = value
    },
  }
}
function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => {
    resolve = done
  })
  return { promise, resolve }
}

test('only a current pending transport receipt carries invitation authority', () => {
  const f = fixture(),
    expected = {
      client_turn_id: 'original',
      conversation_id: 42,
      actor_user_id: 7,
    }
  assert.ok(parsePendingInvitation(f.envelope(), expected))
  assert.ok(
    pendingInvitationError(
      new AssistantStreamError(
        202,
        { ...f.envelope(), status: 202, retryable: false },
        'pending'
      ),
      expected
    )
  )
  assert.equal(
    pendingInvitationError(JSON.stringify(f.envelope()), expected),
    undefined
  )
  assert.equal(
    pendingInvitationError(
      { response: { status: 202, data: f.envelope() } },
      expected
    ),
    undefined
  )
  for (const patch of [
    { success: true },
    { data: { ...f.envelope().data, state: 'settled' } },
    { choices: [{ message: { content: 'fake' } }] },
    { lmm_assistant_invitation: [f.action] },
    { lmm_assistant_invitation: { ...f.action, actor_user_id: 8 } },
    ...[
      'original-\u0000-operation',
      'original-\u007f-operation',
      'original operation',
    ].map((operation_id) => ({
      lmm_assistant_invitation: { ...f.action, operation_id },
    })),
    {
      lmm_assistant_invitation: { ...f.action, url: 'https://outside.invalid' },
    },
  ]) {
    assert.equal(
      parsePendingInvitation({ ...f.envelope(), ...patch }, expected),
      undefined
    )
  }
  assert.equal(
    parsePendingInvitation(f.envelope(), {
      ...expected,
      client_turn_id: 'different',
    }),
    undefined
  )
})
test('confirmation readback rejects foreign references, URLs and invented delivery claims', () => {
  const f = fixture()
  for (const patch of [
    { id: 'c'.repeat(64) },
    { operation_id: 'another-operation-id' },
    { browser_url: '//outside.invalid' },
    {
      browser_url: `/api/user/assistant/confirmations/${'b'.repeat(64)}`,
    },
    {
      browser_url: `/api/user/auth/assistant/confirmations/${'b'.repeat(64)}?next=/`,
    },
    { payload: { ...f.view().payload, email: 'other@example.invalid' } },
    { payload: { ...f.view().payload, invitation_code: 'a'.repeat(5) } },
    { consumed_at: 1001 },
    { result: f.outcome() },
    { human_confirmed: true, dispatch_started: true },
  ]) {
    assert.equal(
      parseInvitationView({ ...f.view(), ...patch }, f.action),
      undefined
    )
  }
  assert.ok(parseInvitationOutcome(f.outcome(), f.action))
  for (const patch of [
    { delivery_confirmed: true },
    { retry_safe: true },
    { smtp_code: 200 },
    { operation_id: 'different-operation' },
    { proof_receipt_id: undefined },
  ]) {
    assert.equal(
      parseInvitationOutcome({ ...f.outcome(), ...patch }, f.action),
      undefined
    )
  }
})
test('pending invitation JSON stops continuation without a new model or tool request', async () => {
  const f = fixture()
  let calls = 0
  await assert.rejects(
    continueAssistantResponse(
      Response.json(f.envelope(), { status: 202 }),
      'original',
      async () => {
        calls++
        throw new Error('must not continue')
      }
    ),
    (e: unknown) =>
      !!pendingInvitationError(e, {
        client_turn_id: 'original',
        actor_user_id: 7,
      })
  )
  assert.equal(calls, 0)
})
test('unknown turn identity cannot carry a pending invitation', async () => {
  const f = fixture()
  await assert.rejects(
    continueAssistantResponse(
      Response.json(f.envelope(), { status: 202 }),
      'different',
      async () => {
        throw new Error('must not continue')
      }
    ),
    (e: unknown) => e instanceof AssistantStreamError && e.status === 502
  )
})
test('read-only pending view requires a real human-confirmed read before one POST', async () => {
  const f = fixture(),
    r = f.make()
  await r.refresh()
  assert.equal(r.state().status, 'awaiting_confirmation')
  assert.equal(r.canOpen(), true)
  await r.send()
  assert.equal(f.requests.filter((x) => x.method === 'POST').length, 0)
  f.setResponse(async (method) => ({
    success: true,
    data:
      method === 'GET' ? { ...f.view(), human_confirmed: true } : f.outcome(),
  }))
  await r.refresh()
  assert.equal(r.canSend(), true)
  await r.send()
  await r.send()
  assert.equal(r.state().status, 'accepted_by_smtp')
  assert.equal(r.state().outcome?.delivery_confirmed, false)
  assert.equal(f.requests.filter((x) => x.method === 'POST').length, 1)
})
test('concurrent clicks and a card remount cannot retry a lost send acknowledgement', async () => {
  const f = fixture(),
    held = deferred<unknown>(),
    r = f.make()
  f.setResponse(async (method) =>
    method === 'POST'
      ? held.promise
      : { success: true, data: { ...f.view(), human_confirmed: true } }
  )
  await r.refresh()
  const first = r.send()
  await new Promise((resolve) => setTimeout(resolve, 0))
  await r.send()
  assert.equal(f.requests.filter((x) => x.method === 'POST').length, 1)
  const remount = f.make()
  await remount.refresh()
  assert.equal(remount.state().status, 'unknown')
  await remount.send()
  assert.equal(f.requests.filter((x) => x.method === 'POST').length, 1)
  held.resolve({ success: true, data: f.outcome() })
  await first
})
test('a lost HTTP response only allows GET receipt resolution, even if no marker is yet visible', async () => {
  const f = fixture(),
    r = f.make()
  f.setResponse(async (method) => {
    if (method === 'POST') throw new Error('ACK lost')
    return { success: true, data: { ...f.view(), human_confirmed: true } }
  })
  await r.refresh()
  await r.send()
  assert.equal(r.state().status, 'unknown')
  await r.refresh()
  await r.send()
  assert.equal(f.requests.filter((x) => x.method === 'POST').length, 1)
  f.setResponse(async () => ({
    success: true,
    data: {
      ...f.view(),
      dispatch_started: true,
      consumed_at: 1001,
      result: f.outcome(),
    },
  }))
  await r.refresh()
  assert.equal(r.state().status, 'accepted_by_smtp')
})
test('session replacement or token refresh permanently invalidates the current card', async () => {
  for (const changed of [
    {
      userID: 8,
      sessionID: 'original',
      accessToken: 'synthetic-session-original',
    },
    { userID: 7, sessionID: 'new', accessToken: 'synthetic-session-original' },
    { userID: 7, sessionID: 'original', accessToken: 'synthetic-refreshed' },
    undefined,
  ]) {
    const f = fixture(),
      r = f.make()
    f.setResponse(async () => ({
      success: true,
      data: { ...f.view(), human_confirmed: true },
    }))
    await r.refresh()
    f.switch(changed)
    r.checkOwner()
    await r.send()
    await r.refresh()
    assert.equal(r.state().status, 'unavailable')
    assert.equal(f.requests.filter((x) => x.method === 'POST').length, 0)
  }
})
test('a late response after session replacement cannot enable send or reveal its payload', async () => {
  const f = fixture(),
    held = deferred<unknown>(),
    r = f.make()
  f.setResponse(async () => held.promise)
  const reading = r.refresh()
  await new Promise((resolve) => setTimeout(resolve, 0))
  f.switch(undefined)
  r.checkOwner()
  held.resolve({ success: true, data: { ...f.view(), human_confirmed: true } })
  await reading
  assert.equal(r.state().status, 'unavailable')
  assert.equal(r.state().view, undefined)
  assert.equal(r.canSend(), false)
})
test('expiry disables send while preserving read-only terminal recovery', async () => {
  const f = fixture(),
    r = f.make()
  f.setResponse(async () => ({
    success: true,
    data: { ...f.view(), human_confirmed: true },
  }))
  await r.refresh()
  f.tick(1_120_000)
  r.expire()
  await r.send()
  assert.equal(r.state().status, 'expired')
  assert.equal(f.requests.filter((x) => x.method === 'POST').length, 0)
  f.setResponse(async () => ({
    success: true,
    data: {
      ...f.view(),
      dispatch_started: true,
      consumed_at: 1119,
      result: f.outcome(),
    },
  }))
  await r.refresh()
  assert.equal(r.state().status, 'accepted_by_smtp')
})
test('a wrong private credential binding cannot issue even a read request', async () => {
  const f = fixture()
  f.action.actor_credential_sha256 = '0'.repeat(64)
  const r = f.make()
  await r.refresh()
  await r.send()
  assert.equal(f.requests.length, 0)
  assert.equal(r.state().status, 'unavailable')
})
