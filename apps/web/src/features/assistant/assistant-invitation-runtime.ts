/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import {
  parseInvitationOutcome,
  parseInvitationSuccess,
  parseInvitationView,
  type InvitationAction,
  type InvitationOutcome,
  type InvitationView,
} from './assistant-invitation-contract'

export type InvitationOwner = {
  userID: number
  sessionID: string
  accessToken: string
}
export type InvitationStatus =
  | 'loading'
  | 'awaiting_confirmation'
  | 'ready'
  | 'sending'
  | 'accepted_by_smtp'
  | 'rejected'
  | 'not_dispatched'
  | 'unknown'
  | 'expired'
  | 'unavailable'
export type InvitationState = {
  status: InvitationStatus
  view?: InvitationView
  outcome?: InvitationOutcome
}
export type InvitationAdapter = {
  owner: () => InvitationOwner | undefined
  request: (
    method: 'GET' | 'POST',
    path: string,
    signal: AbortSignal
  ) => Promise<unknown>
}
export const sameInvitationOwner = (
  a: InvitationOwner | undefined,
  b: InvitationOwner | undefined
) =>
  !!a &&
  !!b &&
  a.userID === b.userID &&
  a.sessionID === b.sessionID &&
  a.accessToken === b.accessToken

// In-memory IDs only. Remounting a card cannot retry a submission with lost ACK.
// No credential, recipient, action envelope or receipt is saved in browser storage.
const attemptedInvitations = new Set<string>()
export function createInvitationRuntime(
  input: InvitationAction,
  adapter: InvitationAdapter,
  now = () => Date.now(),
  attempts = attemptedInvitations
) {
  const action = { ...input }
  const ownerAtCreation = adapter.owner()
  const owner = ownerAtCreation ? { ...ownerAtCreation } : undefined
  const lifetime = new AbortController()
  const listeners = new Set<() => void>()
  const path = `/api/assistant/services/confirmations/${action.confirmation_id}`
  let state: InvitationState = { status: 'loading' }
  let invalidated = false
  let busy = false
  let verified = false
  const publish = (next: InvitationState) => {
    state = next
    for (const listener of listeners) listener()
    return state
  }
  const invalidate = () => {
    invalidated = true
    lifetime.abort()
    publish({ status: 'unavailable' })
  }
  const sameCurrent = () =>
    !invalidated &&
    !lifetime.signal.aborted &&
    sameInvitationOwner(owner, adapter.owner())
  const current = () => {
    if (invalidated || lifetime.signal.aborted) return false
    if (!sameInvitationOwner(owner, adapter.owner())) {
      invalidate()
      return false
    }
    return true
  }
  async function valid(): Promise<boolean> {
    if (
      !current() ||
      !owner ||
      !owner.accessToken ||
      owner.userID !== action.actor_user_id
    ) {
      return false
    }
    if (!verified) {
      if (action.expires_at * 1000 > now() + 300_000) return false
      try {
        const digest = await globalThis.crypto.subtle.digest(
          'SHA-256',
          new TextEncoder().encode(owner.accessToken)
        )
        const encoded = Array.from(new Uint8Array(digest), (byte) =>
          byte.toString(16).padStart(2, '0')
        ).join('')
        if (encoded !== action.actor_credential_sha256 || !current()) {
          return false
        }
        verified = true
      } catch {
        return false
      }
    }
    return current()
  }
  function readState(view: InvitationView): InvitationState {
    if (view.result) {
      return { status: view.result.status, view, outcome: view.result }
    }
    if (view.dispatch_started || attempts.has(action.confirmation_id)) {
      return { status: 'unknown', view }
    }
    if (view.expires_at * 1000 <= now()) return { status: 'expired', view }
    return {
      status: view.human_confirmed ? 'ready' : 'awaiting_confirmation',
      view,
    }
  }
  async function refresh(): Promise<InvitationState> {
    if (busy) return state
    busy = true
    const previous = state
    publish({ ...state, status: 'loading' })
    try {
      if (!(await valid())) {
        invalidate()
        return state
      }
      const raw = await adapter.request('GET', path, lifetime.signal)
      if (!current()) return state
      const view = parseInvitationView(parseInvitationSuccess(raw), action)
      if (!view) throw new Error('Invalid invitation receipt')
      return publish(readState(view))
    } catch {
      if (!current()) return state
      return publish({
        ...previous,
        status:
          attempts.has(action.confirmation_id) ||
          previous.view?.dispatch_started
            ? 'unknown'
            : 'unavailable',
      })
    } finally {
      busy = false
      publish({ ...state })
    }
  }
  const canOpen = () =>
    sameCurrent() &&
    !busy &&
    verified &&
    state.status === 'awaiting_confirmation' &&
    !!state.view &&
    state.view.expires_at * 1000 > now() &&
    !attempts.has(action.confirmation_id)
  const canSend = () =>
    sameCurrent() &&
    !busy &&
    verified &&
    state.status === 'ready' &&
    state.view?.human_confirmed === true &&
    state.view.expires_at * 1000 > now() &&
    !attempts.has(action.confirmation_id) &&
    attempts.size < 1024
  async function send(): Promise<InvitationState> {
    if (!canSend()) return state
    busy = true
    const view = state.view
    try {
      if (!(await valid())) {
        invalidate()
        return state
      }
      if (!view || view.expires_at * 1000 <= now()) {
        return publish({ status: 'expired', view })
      }
      // Set before any request. Neither an exception nor a later read clears it.
      attempts.add(action.confirmation_id)
      publish({ status: 'sending', view })
      const raw = await adapter.request(
        'POST',
        `${path}/apply`,
        lifetime.signal
      )
      if (!current()) return state
      const outcome = parseInvitationOutcome(
        parseInvitationSuccess(raw),
        action
      )
      if (!outcome) throw new Error('Invalid invitation outcome')
      return publish({ status: outcome.status, view, outcome })
    } catch {
      if (!current()) return state
      return publish({ status: 'unknown', view })
    } finally {
      busy = false
      publish({ ...state })
    }
  }
  function expire() {
    if (!current() || busy) return
    if (
      (state.status === 'ready' || state.status === 'awaiting_confirmation') &&
      action.expires_at * 1000 <= now()
    ) {
      publish({ ...state, status: 'expired' })
    }
  }
  return {
    refresh,
    send,
    canOpen,
    canSend,
    expire,
    invalidate,
    state: () => state,
    active: sameCurrent,
    checkOwner: current,
    subscribe: (listener: () => void) => {
      listeners.add(listener)
      return () => {
        listeners.delete(listener)
      }
    },
  }
}
