/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import {
  parseAssistantUIPreferenceAction,
  uiPreferenceUndoPatch,
  type AssistantUIPreferenceAction,
  type UIPreferencePatch,
  type UIPreferences,
} from './assistant-ui-preferences-contract'

export type UIPreferenceOwner = {
  userID: number
  sessionID: string
  // In-memory only. Never persisted, logged or returned in a receipt.
  accessToken?: string
}
export type UIPreferenceAdapter = {
  owner: () => UIPreferenceOwner | undefined
  read: () => UIPreferences
  save: (patch: UIPreferencePatch, signal: AbortSignal) => Promise<void>
  preview: (patch: UIPreferencePatch) => () => void
}
export type UIPreferenceReceipt = {
  status:
    | 'applying'
    | 'applied'
    | 'previewing'
    | 'restored'
    | 'unchanged'
    | 'failed'
    | 'unavailable'
  canUndo: boolean
}

type Undo = {
  id: string
  owner: UIPreferenceOwner
  before: UIPreferences
  patch: UIPreferencePatch
  stop?: () => void
}
export const sameUIPreferenceOwner = (
  a: UIPreferenceOwner | undefined,
  b: UIPreferenceOwner | undefined
) =>
  !!a &&
  !!b &&
  a.userID === b.userID &&
  a.sessionID === b.sessionID &&
  a.accessToken === b.accessToken

async function actionMatchesOwner(
  action: AssistantUIPreferenceAction,
  owner: UIPreferenceOwner
): Promise<boolean> {
  if (action.actor_user_id !== owner.userID) return false
  if (action.actor_session_id !== undefined) {
    return action.actor_session_id === owner.sessionID
  }
  if (!owner.accessToken) return false
  try {
    const digest = await globalThis.crypto.subtle.digest(
      'SHA-256',
      new TextEncoder().encode(owner.accessToken)
    )
    const encoded = Array.from(new Uint8Array(digest), (byte) =>
      byte.toString(16).padStart(2, '0')
    ).join('')
    return encoded === action.actor_credential_sha256
  } catch {
    return false
  }
}

export function createUIPreferenceRuntime(
  now = () => Date.now(),
  schedule = (callback: () => void, delay: number): (() => void) => {
    const timer = setTimeout(callback, delay)
    return () => clearTimeout(timer)
  }
) {
  const records = new Map<
    string,
    { expires: number; owner: UIPreferenceOwner; receipt: UIPreferenceReceipt }
  >()
  const listeners = new Set<() => void>()
  let lastUndo: Undo | undefined
  let busy = false
  let lastPreviewAt = Number.NEGATIVE_INFINITY

  const publish = (id: string, receipt: UIPreferenceReceipt) => {
    const entry = records.get(id)
    if (entry) {
      entry.receipt = receipt
      for (const listener of listeners) listener()
    }
    return receipt
  }
  const discardUndo = () => {
    const previous = lastUndo
    lastUndo = undefined
    previous?.stop?.()
    if (previous) {
      const receipt = records.get(previous.id)?.receipt
      if (receipt) publish(previous.id, { ...receipt, canUndo: false })
    }
  }

  async function undo(
    adapter: UIPreferenceAdapter,
    signal: AbortSignal,
    expectedID?: string
  ): Promise<UIPreferenceReceipt> {
    const saved = lastUndo
    if (
      busy ||
      signal.aborted ||
      !saved ||
      (expectedID !== undefined && saved.id !== expectedID) ||
      !sameUIPreferenceOwner(adapter.owner(), saved.owner)
    ) {
      return { status: 'unchanged', canUndo: false }
    }
    lastUndo = undefined
    if (saved.stop) {
      saved.stop()
      return publish(saved.id, { status: 'restored', canUndo: false })
    }
    const patch = uiPreferenceUndoPatch(
      saved.before,
      saved.patch,
      adapter.read()
    )
    if (Object.keys(patch).length === 0) {
      return publish(saved.id, { status: 'unchanged', canUndo: false })
    }
    busy = true
    publish(saved.id, { status: 'applying', canUndo: false })
    try {
      await adapter.save(patch, signal)
      if (
        signal.aborted ||
        !sameUIPreferenceOwner(adapter.owner(), saved.owner)
      ) {
        throw new Error('session changed')
      }
      return publish(saved.id, { status: 'restored', canUndo: false })
    } catch {
      // Never automatically repeat an uncertain profile write.
      return publish(saved.id, { status: 'failed', canUndo: false })
    } finally {
      busy = false
    }
  }

  async function run(
    value: AssistantUIPreferenceAction,
    adapter: UIPreferenceAdapter,
    signal: AbortSignal
  ): Promise<UIPreferenceReceipt> {
    const action = parseAssistantUIPreferenceAction(value)
    const unavailable: UIPreferenceReceipt = {
      status: 'unavailable',
      canUndo: false,
    }
    if (!action) return unavailable
    // Keep IDs until their server expiry: eviction must never enable a replay.
    for (const [id, entry] of records) {
      if (entry.expires < now()) records.delete(id)
    }
    const currentOwner = adapter.owner()
    const owner = currentOwner ? { ...currentOwner } : undefined
    if (
      signal.aborted ||
      !owner ||
      action.expires_at * 1000 <= now() ||
      action.expires_at * 1000 > now() + 135_000
    ) {
      return unavailable
    }
    // Hash only the captured credential. Recheck after this asynchronous step
    // so account switches and token refresh cannot bind a late response anew.
    if (
      !(await actionMatchesOwner(action, owner)) ||
      signal.aborted ||
      action.expires_at * 1000 <= now() ||
      !sameUIPreferenceOwner(adapter.owner(), owner)
    )
      return unavailable
    const existing = records.get(action.action_id)
    if (existing) {
      return sameUIPreferenceOwner(existing.owner, owner)
        ? existing.receipt
        : unavailable
    }
    if (busy || records.size >= 128) return unavailable
    records.set(action.action_id, {
      expires: action.expires_at * 1000,
      owner,
      receipt: unavailable,
    })
    if (action.tool === 'restore_ui_preferences') {
      return publish(action.action_id, await undo(adapter, signal))
    }
    if (action.temporary && now() - lastPreviewAt < 15_000) {
      return publish(action.action_id, unavailable)
    }
    busy = true
    publish(action.action_id, { status: 'applying', canUndo: false })
    try {
      discardUndo()
      const before = adapter.read()
      const patch = Object.fromEntries(
        Object.entries(action.preview).filter(
          ([key, selected]) => before[key as keyof UIPreferences] !== selected
        )
      ) as UIPreferencePatch
      if (Object.keys(patch).length === 0) {
        return publish(action.action_id, {
          status: 'unchanged',
          canUndo: false,
        })
      }
      if (action.temporary) {
        lastPreviewAt = now()
        const release = adapter.preview(patch)
        let stopped = false
        let cancelTimer = () => {}
        const stop = () => {
          if (stopped) return
          stopped = true
          cancelTimer()
          signal.removeEventListener('abort', stop)
          release()
          if (lastUndo?.id === action.action_id) lastUndo = undefined
          publish(action.action_id, { status: 'restored', canUndo: false })
        }
        lastUndo = { id: action.action_id, owner, before, patch, stop }
        signal.addEventListener('abort', stop, { once: true })
        cancelTimer = schedule(stop, 8_000)
        if (signal.aborted) {
          stop()
          return records.get(action.action_id)?.receipt ?? unavailable
        }
        return publish(action.action_id, {
          status: 'previewing',
          canUndo: true,
        })
      }
      await adapter.save(patch, signal)
      if (signal.aborted || !sameUIPreferenceOwner(adapter.owner(), owner)) {
        throw new Error('session changed')
      }
      lastUndo = { id: action.action_id, owner, before, patch }
      return publish(action.action_id, { status: 'applied', canUndo: true })
    } catch {
      return publish(action.action_id, { status: 'failed', canUndo: false })
    } finally {
      busy = false
    }
  }

  return {
    run,
    undo,
    receipt: (id: string, owner?: UIPreferenceOwner) => {
      const entry = records.get(id)
      return entry && (!owner || sameUIPreferenceOwner(entry.owner, owner))
        ? entry.receipt
        : undefined
    },
    subscribe: (listener: () => void) => {
      listeners.add(listener)
      return () => {
        listeners.delete(listener)
      }
    },
  }
}

// One instance across the page prevents mobile/desktop mounts and repeated
// stream delivery from applying the same action twice. Nothing is persisted.
export const assistantUIPreferenceRuntime = createUIPreferenceRuntime()
