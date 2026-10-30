/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

import {
  CommerceFailure,
  commerceRequest,
  exactUnits,
  nativeId,
  nativeToken,
  operationKey,
  positiveUnits,
  type CommerceScope,
} from '@/features/commerce/native-client'

export const campaignSpecSchema = z
  .object({
    kind: z.enum(['redemption', 'checkin', 'envelope']),
    title: z.string().trim().min(1).max(160),
    award_units: positiveUnits,
    budget_units: positiveUnits,
    max_claims: z.number().int().min(1).max(1000000),
    max_claims_per_user: z.number().int().min(1).max(10000),
    starts_at: z.number().int().nonnegative(),
    ends_at: z.number().int().positive(),
    enabled: z.boolean(),
  })
  .strict()
export type CampaignSpec = z.infer<typeof campaignSpecSchema>
export const campaignSchema = z
  .object({
    id: nativeToken,
    revision: nativeId,
    spec: campaignSpecSchema,
    spent_units: exactUnits,
    reserved_units: exactUnits,
    claims: z.number().int().nonnegative(),
    reserved_claims: z.number().int().nonnegative(),
  })
  .strict()
export type NativeCampaign = z.infer<typeof campaignSchema>
export const saveCampaignSchema = z
  .object({
    operation_key: operationKey,
    expected_revision: nativeId.nullable(),
    spec: campaignSpecSchema,
  })
  .strict()
export const issueRequestSchema = z
  .object({
    operation_key: operationKey,
    quantity: z.number().int().min(1).max(100),
  })
  .strict()
export const issueSchema = z
  .object({
    id: nativeToken,
    campaign_id: nativeToken,
    codes: z
      .array(
        z
          .object({
            id: nativeToken,
            code: z.string().regex(/^lmmr_[A-Za-z0-9_-]{43}$/),
          })
          .strict()
      )
      .min(1)
      .max(100),
  })
  .strict()
export type NativeIssue = z.infer<typeof issueSchema>
export const codeStatusSchema = z
  .object({ id: nativeToken, revoked: z.boolean(), claimed: z.boolean() })
  .strict()
export type NativeCodeStatus = z.infer<typeof codeStatusSchema>
export const rewardSchema = z
  .object({
    id: nativeToken,
    campaign_id: nativeToken,
    amount_units: positiveUnits,
    journal_id: nativeId,
    claimed_at: z.number().int().positive(),
  })
  .strict()
export type NativeReward = z.infer<typeof rewardSchema>
const base = '/core/v1/promotions'
export async function listCampaigns(scope: CommerceScope, admin = false) {
  return (
    await commerceRequest(
      scope,
      'get',
      `${base}/${admin ? 'admin/' : ''}campaigns`,
      z.object({
        contract: z.literal('core.promotions.v1'),
        campaigns: z.array(campaignSchema),
      })
    )
  ).campaigns
}
export function saveCampaign(
  scope: CommerceScope,
  id: string | null,
  request: z.infer<typeof saveCampaignSchema>
) {
  saveCampaignSchema.parse(request)
  return commerceRequest(
    scope,
    id ? 'put' : 'post',
    `${base}/admin/campaigns${id ? `/${id}` : ''}`,
    campaignSchema,
    request
  )
}
export async function issueCodes(
  scope: CommerceScope,
  campaign: NativeCampaign,
  request: z.infer<typeof issueRequestSchema>
) {
  issueRequestSchema.parse(request)
  const result = await commerceRequest(
    scope,
    'post',
    `${base}/admin/campaigns/${campaign.id}/codes`,
    issueSchema,
    request
  )
  if (
    result.campaign_id !== campaign.id ||
    result.codes.length !== request.quantity
  ) {
    throw new CommerceFailure('INVALID_RESPONSE')
  }
  return result
}
export function listCodes(scope: CommerceScope, campaign: NativeCampaign) {
  return commerceRequest(
    scope,
    'get',
    `${base}/admin/campaigns/${campaign.id}/codes`,
    z.array(codeStatusSchema)
  )
}
export async function revokeCode(scope: CommerceScope, code: NativeCodeStatus) {
  const result = await commerceRequest(
    scope,
    'post',
    `${base}/admin/codes/${code.id}/revoke`,
    codeStatusSchema
  )
  if (result.id !== code.id || !result.revoked || result.claimed) {
    throw new CommerceFailure('INVALID_RESPONSE')
  }
  return result
}
export async function redeemCode(
  scope: CommerceScope,
  operation_key: string,
  code: string
) {
  operationKey.parse(operation_key)
  if (!/^lmmr_[A-Za-z0-9_-]{43}$/.test(code)) {
    throw new CommerceFailure('PROMOTION_INVALID', false)
  }
  return commerceRequest(scope, 'post', `${base}/redemptions`, rewardSchema, {
    operation_key,
    code,
  })
}
export async function claimCampaign(
  scope: CommerceScope,
  campaign: NativeCampaign,
  operation_key: string
) {
  if (campaign.spec.kind === 'redemption') {
    throw new CommerceFailure('PROMOTION_INVALID', false)
  }
  operationKey.parse(operation_key)
  const result = await commerceRequest(
    scope,
    'post',
    `${base}/campaigns/${campaign.id}/${campaign.spec.kind}`,
    rewardSchema,
    { operation_key }
  )
  if (
    result.campaign_id !== campaign.id ||
    result.amount_units !== campaign.spec.award_units
  ) {
    throw new CommerceFailure('INVALID_RESPONSE')
  }
  return result
}
export async function rewardHistory(scope: CommerceScope) {
  return (
    await commerceRequest(
      scope,
      'get',
      `${base}/history`,
      z.object({
        contract: z.literal('core.promotions.v1'),
        claims: z.array(rewardSchema),
      })
    )
  ).claims
}
export async function redemptionDigest(code: string) {
  const bytes = await crypto.subtle.digest(
    'SHA-256',
    new TextEncoder().encode(code)
  )
  return Array.from(new Uint8Array(bytes), (value) =>
    value.toString(16).padStart(2, '0')
  ).join('')
}
