/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

import {
  buildRegistrationResult,
  prepareCredentialCreationOptions,
} from '@/lib/passkey'

const encoded = z
  .string()
  .min(1)
  .max(4096)
  .regex(/^[A-Za-z0-9_-]+$/)
const publicOptions = z.strictObject({
  publicKey: z.strictObject({
    rp: z.strictObject({ id: z.string().min(1), name: z.string().min(1) }),
    user: z.strictObject({
      id: encoded,
      name: z.string().min(1).max(80),
      displayName: z.string().min(1).max(80),
    }),
    challenge: encoded,
    pubKeyCredParams: z
      .array(
        z.strictObject({ type: z.literal('public-key'), alg: z.number().int() })
      )
      .min(1),
    timeout: z.number().int().positive().max(300000),
    attestation: z.literal('none'),
    authenticatorSelection: z.strictObject({
      requireResidentKey: z.boolean(),
      residentKey: z.enum(['discouraged', 'preferred', 'required']),
      userVerification: z.literal('required'),
    }),
    extensions: z.strictObject({
      credProps: z.boolean(),
      credentialProtectionPolicy: z.literal('userVerificationRequired'),
      enforceCredentialProtectionPolicy: z.boolean(),
      uvm: z.boolean(),
    }),
    excludeCredentials: z
      .array(
        z.strictObject({
          type: z.literal('public-key'),
          id: encoded,
          transports: z.array(z.string()).optional(),
        })
      )
      .optional(),
  }),
})
const begin = z.strictObject({
  protocol: z.literal('core.trust-policy-owner.v1'),
  // This acknowledges BEGIN only. It conveys no registration approval.
  status: z.literal('confirmed'),
  data: z.strictObject({
    enrollment: z.string().regex(/^[0-9a-f]{64}$/),
    options: publicOptions,
  }),
})
export type PublicRegistration = z.infer<typeof begin>['data']
export const REGISTRATION_MAX_BYTES = 100000

export function parsePublicRegistration(
  raw: string,
  origin: string
): PublicRegistration {
  if (new TextEncoder().encode(raw).length > REGISTRATION_MAX_BYTES) {
    throw new Error('Invalid public registration options')
  }
  const parsed = begin.safeParse(JSON.parse(raw))
  if (!parsed.success) throw new Error('Invalid public registration options')
  const page = new URL(origin)
  if (
    page.origin !== origin ||
    (page.protocol !== 'https:' &&
      !(page.protocol === 'http:' && page.hostname === 'localhost')) ||
    page.hostname !== parsed.data.data.options.publicKey.rp.id
  ) {
    throw new Error('Registration origin mismatch')
  }
  // Validate encoded fields through the shared converter before showing a creation action.
  prepareCredentialCreationOptions(parsed.data.data.options)
  return parsed.data.data
}

export async function createPublicRegistration(
  input: PublicRegistration,
  signal: AbortSignal
): Promise<string> {
  if (
    !window.isSecureContext ||
    typeof navigator.credentials?.create !== 'function'
  ) {
    throw new Error('Secure WebAuthn browser required')
  }
  // Recheck the current page and copy the public input before yielding to the browser.
  const checked = parsePublicRegistration(
    JSON.stringify({
      protocol: 'core.trust-policy-owner.v1',
      status: 'confirmed',
      data: input,
    }),
    window.location.origin
  )
  const publicKey = prepareCredentialCreationOptions(checked.options)
  const credential = (await navigator.credentials.create({
    publicKey,
    signal,
  })) as PublicKeyCredential | null
  if (signal.aborted) throw new Error('Registration cancelled')
  const response = buildRegistrationResult(credential)
  if (!response) throw new Error('Registration cancelled')
  // Leave room for the actual CLI review request envelope and enrollment ID.
  const request = JSON.stringify({
    operation: 'review_registration',
    enrollment: checked.enrollment,
    response,
  })
  if (new TextEncoder().encode(request).length > REGISTRATION_MAX_BYTES) {
    throw new Error('Registration response exceeds CLI limit')
  }
  return `${JSON.stringify(response, null, 2)}\n`
}
