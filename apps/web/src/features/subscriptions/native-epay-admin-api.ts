/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

import {
  CommerceFailure,
  commerceRequest,
  currentCommerceScope,
  nativeId,
  operationKey,
  type CommerceScope,
} from '@/features/commerce/native-client'
import { nativeChannelSchema } from '@/features/wallet/lib/native-checkout'
import { ROLE } from '@/lib/roles'
import { useAuthStore } from '@/stores/auth-store'

import { planSchema } from './native-api'
import { epayPriceSchema } from './native-epay-api'

const writablePriceSchema = epayPriceSchema
  .pick({
    plan_id: true,
    plan_revision: true,
    channel_id: true,
    currency: true,
    amount_minor: true,
  })
  .strict()
export const saveEPayPriceSchema = z
  .object({ operation_key: operationKey, price: writablePriceSchema })
  .strict()
export type SaveEPayPrice = z.infer<typeof saveEPayPriceSchema>
export const pendingEPayPriceSchema = z
  .object({ plan: planSchema, request: saveEPayPriceSchema })
  .strict()
  .refine(
    (value) =>
      value.plan.id === value.request.price.plan_id &&
      value.plan.revision === value.request.price.plan_revision
  )
export type PendingEPayPrice = z.infer<typeof pendingEPayPriceSchema>
const epayChannelSchema = z.object({
  id: z
    .string()
    .regex(/^[1-9][0-9]*$/)
    .transform(Number)
    .pipe(nativeId),
  provider: z.literal('epay'),
  currency: z.enum(['CNY', 'LDC']),
  environment: z.enum(['test', 'live']),
  payment_type: z.string().optional(),
})
export type EPayAdminChannel = z.infer<typeof epayChannelSchema>
function assertAdmin(scope: CommerceScope) {
  if (!currentCommerceScope(scope)) throw new CommerceFailure('SESSION_CHANGED')
  if ((useAuthStore.getState().auth.user?.role ?? 0) < ROLE.SUPER_ADMIN) {
    throw new CommerceFailure('FORBIDDEN', false)
  }
}
export async function listAdminEPayPrices(
  scope: CommerceScope,
  planId: number
) {
  assertAdmin(scope)
  nativeId.parse(planId)
  const result = await commerceRequest(
    scope,
    'get',
    `/core/v1/subscriptions/admin/plans/${planId}/epay-prices`,
    z.object({ prices: z.array(epayPriceSchema) })
  )
  assertAdmin(scope)
  if (result.prices.some((price) => price.plan_id !== planId)) {
    throw new CommerceFailure('PRICE_MISMATCH')
  }
  return result.prices
}
export async function listAdminEPayChannels(scope: CommerceScope) {
  assertAdmin(scope)
  const result = await commerceRequest(
    scope,
    'get',
    '/api/user/payments/channels',
    z.object({
      success: z.literal(true),
      data: z.object({
        available: z.boolean(),
        channels: z.array(nativeChannelSchema),
      }),
    })
  )
  assertAdmin(scope)
  if (!result.data.available) return []
  return result.data.channels
    .filter(
      (channel) =>
        channel.provider === 'epay' &&
        (channel.currency === 'CNY' || channel.currency === 'LDC')
    )
    .map((channel) => epayChannelSchema.parse(channel))
}
export async function saveAdminEPayPrice(
  scope: CommerceScope,
  input: SaveEPayPrice
) {
  assertAdmin(scope)
  const request = saveEPayPriceSchema.parse(input)
  const result = await commerceRequest(
    scope,
    'post',
    `/core/v1/subscriptions/admin/plans/${request.price.plan_id}/epay-prices`,
    writablePriceSchema,
    request
  )
  assertAdmin(scope)
  if (JSON.stringify(result) !== JSON.stringify(request.price)) {
    throw new CommerceFailure('PRICE_MISMATCH')
  }
  return result
}
