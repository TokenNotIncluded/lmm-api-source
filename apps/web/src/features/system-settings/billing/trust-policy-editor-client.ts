/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

import { api } from '@/lib/api'
import {
  buildAssertionResult,
  prepareCredentialRequestOptions,
} from '@/lib/passkey'
import { useAuthStore } from '@/stores/auth-store'

import {
  trustLevelBenefitsSchema,
  type TrustLevelBenefitsConfig,
} from './trust-level-benefits-schema'

const positive = z.number().int().positive().max(Number.MAX_SAFE_INTEGER)
const integer = z.union([positive, z.bigint().positive()]).transform(String)
const ratio = z
  .strictObject({ numerator: integer, denominator: integer })
  .refine(
    (v) =>
      BigInt(v.numerator) <= BigInt(v.denominator) &&
      BigInt(v.denominator) <= 10n ** 18n
  )
export const trustNumericPolicySchema = z
  .strictObject({
    version: z.literal(1),
    paid_activation_enabled: z.boolean(),
    decay_period_days: z.number().int().min(0).max(3650),
    tiers: z
      .array(
        z.strictObject({
          level: z.number().int().min(0).max(4),
          min_paid_credits: z
            .number()
            .int()
            .min(0)
            .max(Number.MAX_SAFE_INTEGER),
          discount: ratio,
        })
      )
      .length(5),
    admin_discount: ratio,
    superadmin_discount: ratio,
  })
  .refine((v) =>
    v.tiers.every(
      (tier, index) =>
        tier.level === index &&
        (index === 0
          ? tier.min_paid_credits === 0
          : tier.min_paid_credits > v.tiers[index - 1].min_paid_credits)
    )
  )
const consolePolicy = trustLevelBenefitsSchema.refine(
  (v) => v.role_tiers?.length === 2
)
const hex = z.string().regex(/^[0-9a-f]{64}$/)
export const trustPolicyReadSchema = z.strictObject({
  revision: positive,
  policy: trustNumericPolicySchema,
  console: consolePolicy.nullable(),
  editors: z
    .array(
      z.strictObject({
        editor_id: z.string().min(1).max(200),
        version: positive,
        status: z.enum(['active', 'revoked', 'configuration_mismatch']),
      })
    )
    .refine(
      (editors) =>
        new Set(editors.map((e) => e.editor_id)).size === editors.length
    ),
})
const previewSchema = z.strictObject({
  token: hex,
  preview: z.strictObject({
    actor_user_id: positive,
    session_credential_id: positive,
    base_revision: positive,
    current: trustNumericPolicySchema,
    current_console: consolePolicy.nullable(),
    proposed: trustNumericPolicySchema,
    proposed_console: consolePolicy,
    digest: hex,
  }),
})
const resolutionSchema = z
  .strictObject({
    state: z.enum(['confirmed', 'pending', 'unknown', 'expired']),
    revision: positive.nullable(),
  })
  .refine((v) =>
    v.state === 'confirmed' ? v.revision !== null : v.revision === null
  )
export type TrustNumericPolicy = z.infer<typeof trustNumericPolicySchema>
export type TrustPolicyRead = z.infer<typeof trustPolicyReadSchema>
export type TrustPolicyPreview = z.infer<typeof previewSchema>
export type TrustPolicyResolution = z.infer<typeof resolutionSchema>
export type TrustPolicyScope = { userId: number; sessionId: string }

export function trustPolicyScope(): TrustPolicyScope | null {
  const { user, session } = useAuthStore.getState().auth
  return user?.role === 100 &&
    Number.isSafeInteger(user.id) &&
    user.id > 0 &&
    session?.sid
    ? { userId: user.id, sessionId: session.sid }
    : null
}

// Axios' default JSON decoder rounds u64 ratios before validation. Preserve only
// integer tokens outside strings; all other JSON syntax still goes through JSON.parse.
export function decodeTrustPolicyJson(raw: unknown): unknown {
  if (typeof raw !== 'string') return raw
  const integers: bigint[] = []
  const prefix = '__trust_policy_integer__'
  if (raw.includes(prefix)) throw new Error('Invalid trust policy response')
  const protectedJson = raw.replaceAll(
    /"(?:\\.|[^"\\])*"|-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?/g,
    (token) => {
      if (token.startsWith('"')) {
        if (String(JSON.parse(token)).startsWith(prefix)) {
          throw new Error('Invalid trust policy response')
        }
        return token
      }
      if (!/^-?\d+$/.test(token) || Number.isSafeInteger(Number(token))) {
        return token
      }
      integers.push(BigInt(token))
      return JSON.stringify(`${prefix}${integers.length - 1}`)
    }
  )
  return JSON.parse(protectedJson, (_key, value: unknown) =>
    typeof value === 'string' && value.startsWith(prefix)
      ? integers[Number(value.slice(prefix.length))]
      : value
  )
}

function decimalRatio(value: number) {
  const [mantissa, exponent = '0'] = String(value).toLowerCase().split('e')
  const [whole, fraction = ''] = mantissa.split('.')
  const shift = Number(exponent) - fraction.length
  return {
    numerator: BigInt(whole + fraction) * 10n ** BigInt(Math.max(0, shift)),
    denominator: 10n ** BigInt(Math.max(0, -shift)),
  }
}
function equalsRatio(
  value: number,
  expected: TrustNumericPolicy['admin_discount']
) {
  const actual = decimalRatio(value)
  return (
    actual.numerator * BigInt(expected.denominator) ===
    BigInt(expected.numerator) * actual.denominator
  )
}
export function assertTrustDescription(
  policy: TrustNumericPolicy,
  description: TrustLevelBenefitsConfig
) {
  if (
    policy.version !== description.version ||
    policy.paid_activation_enabled !== description.paid_activation_enabled ||
    policy.decay_period_days !== description.decay_period_days ||
    !description.role_tiers ||
    !policy.tiers.every(
      (tier, i) =>
        tier.min_paid_credits === description.tiers[i].min_paid_credits &&
        equalsRatio(description.tiers[i].discount_ratio, tier.discount)
    ) ||
    !equalsRatio(
      description.role_tiers[0].discount_ratio,
      policy.admin_discount
    ) ||
    !equalsRatio(
      description.role_tiers[1].discount_ratio,
      policy.superadmin_discount
    )
  ) {
    throw new Error('The policy cannot be represented exactly in this editor.')
  }
}
export function trustPolicyDraft(
  read: TrustPolicyRead
): TrustLevelBenefitsConfig {
  if (read.console) {
    assertTrustDescription(read.policy, read.console)
    return structuredClone(read.console)
  }
  const decimal = (r: TrustNumericPolicy['admin_discount']) =>
    Number(r.numerator) / Number(r.denominator)
  const draft: TrustLevelBenefitsConfig = {
    version: 1,
    paid_activation_enabled: read.policy.paid_activation_enabled,
    decay_period_days: read.policy.decay_period_days,
    tiers: read.policy.tiers.map((tier) => ({
      level: tier.level,
      min_paid_credits: tier.min_paid_credits,
      discount_ratio: decimal(tier.discount),
      benefits: [],
    })),
    role_tiers: [
      {
        level: 5,
        role: 10,
        discount_ratio: decimal(read.policy.admin_discount),
        benefits: [],
      },
      {
        level: 6,
        role: 100,
        discount_ratio: decimal(read.policy.superadmin_discount),
        benefits: [],
      },
    ],
  }
  assertTrustDescription(read.policy, draft)
  return draft
}

function freeze<T>(value: T): T {
  if (value && typeof value === 'object') {
    Object.values(value).forEach(freeze)
    Object.freeze(value)
  }
  return value
}

/** One in-memory preview, one challenge attempt. Never stored in a query cache. */
export class TrustPolicyEditorClient {
  private pending: TrustPolicyPreview | null = null
  private challengeAttempted = false
  private confirmationSent = false
  private resolution: TrustPolicyResolution | null = null
  private lifetime = new AbortController()
  constructor(readonly scope: TrustPolicyScope) {}

  private current(renewedSession = false) {
    const current = trustPolicyScope()
    if (
      this.lifetime.signal.aborted ||
      !current ||
      current.userId !== this.scope.userId ||
      (!renewedSession && current.sessionId !== this.scope.sessionId)
    ) {
      throw new Error('The editing session changed. Reload the policy.')
    }
    return current
  }
  private async request<T>(
    path: string,
    schema: z.ZodType<T>,
    body?: unknown,
    renewedSession = false
  ): Promise<T> {
    const scope = this.current(renewedSession)
    const config = {
      authScope: scope,
      disableDuplicate: true,
      skipBusinessError: true,
      skipErrorHandler: true,
      skipAuthRefresh: true,
      signal: this.lifetime.signal,
      timeout: 10000,
      transformResponse: [decodeTrustPolicyJson],
    }
    const response =
      body === undefined
        ? await api.get(path, config)
        : await api.post(path, body, config)
    // Do not display a late response to another account/session.
    const after = this.current(renewedSession)
    if (after.sessionId !== scope.sessionId) {
      throw new Error('The editing session changed. Reload the policy.')
    }
    const result = z
      .object({ success: z.literal(true), data: schema })
      .safeParse(decodeTrustPolicyJson(response.data))
    if (!result.success) throw new Error('Invalid trust policy response')
    return result.data.data
  }
  async read() {
    const read = await this.request(
      '/core/v1/admin/trust-policy',
      trustPolicyReadSchema
    )
    if (read.console) assertTrustDescription(read.policy, read.console)
    return read
  }
  async preview(input: TrustLevelBenefitsConfig) {
    if (this.pending) {
      throw new Error('Resolve or discard the previous review first.')
    }
    const draft = consolePolicy.parse(input)
    const result = await this.request(
      '/api/option/trust-policy/preview',
      previewSchema,
      { value: JSON.stringify(draft) }
    )
    if (
      result.preview.actor_user_id !== this.scope.userId ||
      JSON.stringify(result.preview.proposed_console) !== JSON.stringify(draft)
    ) {
      throw new Error('Invalid trust policy response')
    }
    assertTrustDescription(result.preview.proposed, draft)
    if (result.preview.current_console) {
      assertTrustDescription(
        result.preview.current,
        result.preview.current_console
      )
    }
    this.pending = freeze(result)
    this.challengeAttempted = false
    return this.pending
  }
  get canDiscard() {
    return (
      !this.confirmationSent ||
      this.resolution?.state === 'expired' ||
      this.resolution?.state === 'unknown' ||
      this.resolution?.state === 'confirmed'
    )
  }
  dispose() {
    this.lifetime.abort()
  }
  discard() {
    if (!this.canDiscard) {
      throw new Error('Resolve the original review before starting another.')
    }
    this.pending = null
    this.challengeAttempted = false
    this.confirmationSent = false
    this.resolution = null
  }
  async resolve(): Promise<TrustPolicyResolution> {
    if (!this.pending) throw new Error('No policy review is available.')
    this.resolution = await this.request(
      '/api/option/trust-policy/resolve',
      resolutionSchema,
      { token: this.pending.token },
      true
    )
    if (
      this.resolution.state === 'confirmed' &&
      this.resolution.revision !== this.pending.preview.base_revision + 1
    ) {
      this.resolution = null
      throw new Error('Invalid trust policy response')
    }
    return this.resolution
  }
  async confirm(editor: string): Promise<TrustPolicyResolution> {
    if (!this.pending || this.challengeAttempted) {
      throw new Error(
        'Create and review a new preview before another credential attempt.'
      )
    }
    this.current()
    this.challengeAttempted = true
    const {
      token,
      preview: { digest },
    } = this.pending
    try {
      const challenge = await this.request(
        '/api/option/trust-policy/challenge',
        z.strictObject({ challenge: hex, options: z.unknown() }),
        { token, digest, editor }
      )
      const publicKey = prepareCredentialRequestOptions(challenge.options)
      const credential = (await navigator.credentials.get({
        publicKey,
        signal: this.lifetime.signal,
      })) as PublicKeyCredential | null
      const assertion = buildAssertionResult(credential)
      if (!assertion) throw new Error('Credential confirmation was cancelled.')
      this.current()
      this.confirmationSent = true
      const result = await this.request(
        '/api/option/trust-policy/confirm',
        z.strictObject({ revision: positive }),
        { token, digest, challenge: challenge.challenge, assertion }
      )
      if (result.revision !== this.pending.preview.base_revision + 1) {
        throw new Error('Invalid trust policy response')
      }
      this.resolution = { state: 'confirmed', revision: result.revision }
      return this.resolution
    } catch {
      // No retry of confirm/challenge. The original token is the only resolver.
      return this.resolve()
    }
  }
}
