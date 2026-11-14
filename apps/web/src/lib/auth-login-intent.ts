/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
/** Shared across redirects and tabs; contains no credential or identity data. */
export const INITIAL_LOGIN_INTENT = 'initial'
const key = 'new-api:auth-login-intent'
const initializationKey = 'new-api:auth-login-initialization'
const uuid = /^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$/i
export type IntentStorage = Pick<Storage, 'getItem' | 'setItem'>
let memoryVersion = INITIAL_LOGIN_INTENT
let memoryInitialization: string | null = null

export function readLoginIntent(storage?: Pick<Storage, 'getItem'>): string {
  const value = storage
    ? (storage.getItem(key) ?? INITIAL_LOGIN_INTENT)
    : memoryVersion
  if (!value || value.length > 128) throw new Error('OAuth storage unavailable')
  return value
}

export function invalidateLoginIntent(storage?: IntentStorage): void {
  const version = globalThis.crypto.randomUUID()
  // C can be deleted independently of I. Retiring an existing initialization
  // epoch prevents that deletion from making an old "initial" receipt current.
  // A legacy cancellation before native initialization still does not create I.
  if (readLoginInitialization(storage) !== null) {
    const next = globalThis.crypto.randomUUID()
    if (storage) storage.setItem(initializationKey, next)
    else memoryInitialization = next
    if (readLoginInitialization(storage) !== next) {
      throw new Error('OAuth storage unavailable')
    }
  }
  if (storage) {
    storage.setItem(key, version)
    if (storage.getItem(key) !== version) {
      throw new Error('OAuth storage unavailable')
    }
  } else {
    memoryVersion = version
  }
}

/** Read-only: absence must invalidate an issued ceremony, never reconstruct it. */
export function readLoginInitialization(
  storage?: Pick<Storage, 'getItem'>
): string | null {
  const value = storage
    ? storage.getItem(initializationKey)
    : memoryInitialization
  if (value !== null && !uuid.test(value)) {
    throw new Error('OAuth storage unavailable')
  }
  return value
}

/** Caller must hold the existing Cookie lock; initialization never writes C. */
export function ensureLoginInitialization(storage?: IntentStorage): string {
  const existing = readLoginInitialization(storage)
  if (existing !== null) return existing
  const value = globalThis.crypto.randomUUID()
  if (storage) storage.setItem(initializationKey, value)
  else memoryInitialization = value
  if (readLoginInitialization(storage) !== value) {
    throw new Error('OAuth storage unavailable')
  }
  return value
}

export function formatLoginIntentVersion(
  initialization: string,
  cancellation: string
): string {
  if (
    !uuid.test(initialization) ||
    (cancellation !== INITIAL_LOGIN_INTENT && !uuid.test(cancellation))
  ) {
    throw new Error('OAuth storage unavailable')
  }
  return `lmi1:${initialization}:${cancellation}`
}

export function parseLoginIntentVersion(
  version: unknown
): { initialization: string; cancellation: string } | null {
  if (typeof version !== 'string' || version.length > 128) return null
  const [format, initialization, cancellation, extra] = version.split(':')
  if (
    format !== 'lmi1' ||
    !initialization ||
    !uuid.test(initialization) ||
    !cancellation ||
    (cancellation !== INITIAL_LOGIN_INTENT && !uuid.test(cancellation)) ||
    extra !== undefined
  ) {
    return null
  }
  return { initialization, cancellation }
}

export function isLoginIntentVersionCurrent(
  version: unknown,
  storage?: Pick<Storage, 'getItem'>
): boolean {
  const receipt = parseLoginIntentVersion(version)
  return (
    receipt !== null &&
    readLoginInitialization(storage) === receipt.initialization &&
    readLoginIntent(storage) === receipt.cancellation
  )
}
