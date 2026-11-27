/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import type { UsageLog } from '../data/schema'
import type { LogOtherData } from '../types'
import { safeLogDiagnostic } from './recovery'

// Arbitrary provider prose may contain prompts, URLs or credentials even after
// ordinary token redaction. Only these complete, context-free messages are safe.
const publicMessages = new Set([
  'bad request',
  'invalid request',
  'unauthorized',
  'forbidden',
  'not found',
  'model not found',
  'method not allowed',
  'too many requests',
  'rate limit exceeded',
  'internal server error',
  'bad gateway',
  'service unavailable',
  'gateway timeout',
])
const publicPaths = new Set([
  '/v1/chat/completions',
  '/v1/completions',
  '/v1/responses',
  '/v1/responses/compact',
  '/v1/messages',
  '/v1/messages/count_tokens',
  '/v1/embeddings',
  '/v1/rerank',
  '/v1/images/generations',
  '/v1/images/edits',
  '/v1/audio/speech',
  '/v1/audio/transcriptions',
])

function safeCode(value: unknown): string | undefined {
  if (
    typeof value !== 'string' ||
    !/^[a-zA-Z][a-zA-Z0-9_.-]{0,79}$/.test(value)
  )
    return
  const safe = JSON.parse(
    safeLogDiagnostic(JSON.stringify({ code: value }))
  ) as { code?: string }
  return safe.code
}

export function failureFacts(
  log: UsageLog,
  other: LogOtherData | null,
  isAdmin: boolean
) {
  const observation = isAdmin ? log.relay_diagnostics : undefined
  let code = safeCode(observation?.error_code) ?? safeCode(other?.error_code)
  let message: string | undefined
  let messageWithheld = false
  if (log.content.length <= 12000) {
    try {
      const content = JSON.parse(log.content)
      const error = content?.error ?? content
      code ??= safeCode(error?.code ?? error?.type)
      if (typeof error?.message === 'string' && error.message.trim()) {
        const text = error.message.trim()
        if (publicMessages.has(text.replace(/\.$/, '').toLowerCase()))
          message = text
        else messageWithheld = true
      }
    } catch {
      // Free-form bodies do not supply structured diagnostic fields.
    }
  }
  const path = other?.request_path?.split(/[?#]/, 1)[0]
  const attempts = isAdmin ? log.route_attempts : undefined
  const attempt = attempts?.at(-1)
  const previous = attempts?.at(-2)
  const retryFailure = safeCode(previous?.error_kind)
  return {
    code,
    message,
    messageWithheld,
    path: path && publicPaths.has(path) ? path : undefined,
    protocol: attempt?.protocol,
    inbound: observation?.inbound_protocol ?? attempt?.inbound_protocol,
    stage: safeCode(observation?.error_stage),
    frame: observation?.sse_frame,
    failureKind: safeCode(attempt?.error_kind),
    endReason: safeCode(other?.stream_status?.end_reason),
    observedRetry:
      previous && attempt && retryFailure
        ? `${previous.attempt} → ${attempt.attempt} · ${retryFailure}`
        : undefined,
  }
}
