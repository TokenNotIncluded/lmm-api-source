/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
// Synthetic data only. Shared by the isolated client and DOM tests.
import { useAuthStore } from '@/stores/auth-store'

import type { TrustLevelBenefitsConfig } from './trust-level-benefits-schema'

export const token = 'ab'.repeat(32)
export const digest = 'cd'.repeat(32)
export const challengeId = 'ef'.repeat(32)
export function signIn(userId = 7, sessionId = 'fixture-session', role = 100) {
  useAuthStore.getState().auth.setBundle({
    access_token: 'synthetic-test-only',
    access_expires_at: 9999999999,
    token_type: 'Bearer',
    user: { id: userId, username: 'fixture-editor', role },
    session: {
      sid: sessionId,
      current: true,
      login_method: 'passkey',
      ip: null,
      user_agent: null,
      created_at: 1,
      last_active_at: 1,
      expires_at: 9999999999,
    },
  })
}
export function numericPolicy() {
  return {
    version: 1,
    paid_activation_enabled: true,
    decay_period_days: 30,
    tiers: [0, 1, 2, 3, 4].map((level) => ({
      level,
      min_paid_credits: level * 100,
      discount: { numerator: 100 - level, denominator: 100 },
    })),
    admin_discount: { numerator: 9, denominator: 10 },
    superadmin_discount: { numerator: 85, denominator: 100 },
  }
}
export function consolePolicy(): TrustLevelBenefitsConfig {
  return {
    version: 1,
    paid_activation_enabled: true,
    decay_period_days: 30,
    tiers: [0, 1, 2, 3, 4].map((level) => ({
      level,
      min_paid_credits: level * 100,
      discount_ratio: (100 - level) / 100,
      benefits: [],
    })),
    role_tiers: [
      { level: 5, role: 10, discount_ratio: 0.9, benefits: [] },
      { level: 6, role: 100, discount_ratio: 0.85, benefits: [] },
    ],
  }
}
export function policyRead() {
  return {
    revision: 12,
    policy: numericPolicy(),
    console: null as TrustLevelBenefitsConfig | null,
    editors: [
      { editor_id: 'owner-enrolled-key', version: 1, status: 'active' },
    ],
  }
}
export function previewReply(draft = consolePolicy()) {
  const proposed = numericPolicy()
  proposed.paid_activation_enabled = draft.paid_activation_enabled
  proposed.decay_period_days = draft.decay_period_days
  proposed.tiers.forEach((tier, i) => {
    tier.min_paid_credits = draft.tiers[i].min_paid_credits
  })
  return {
    token,
    preview: {
      actor_user_id: 7,
      session_credential_id: 17,
      base_revision: 12,
      current: numericPolicy(),
      current_console: null as TrustLevelBenefitsConfig | null,
      proposed,
      proposed_console: draft,
      digest,
    },
  }
}
export function challengeReply() {
  return {
    challenge: challengeId,
    options: {
      publicKey: {
        challenge: 'AQID',
        rpId: 'console.example.test',
        allowCredentials: [{ type: 'public-key', id: 'BAUG' }],
        userVerification: 'required',
      },
    },
  }
}
export function assertionCredential(): PublicKeyCredential {
  const bytes = (values: number[]) => new Uint8Array(values).buffer
  return {
    id: 'BAUG',
    rawId: bytes([4, 5, 6]),
    type: 'public-key',
    authenticatorAttachment: 'cross-platform',
    response: {
      authenticatorData: bytes([7, 8]),
      clientDataJSON: bytes([9, 10]),
      signature: bytes([11, 12]),
      userHandle: null,
    },
    getClientExtensionResults: () => ({}),
  } as unknown as PublicKeyCredential
}
export const envelope = (data: unknown) => ({ data: { success: true, data } })
