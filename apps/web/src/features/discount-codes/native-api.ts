/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { z } from 'zod'

import {
  commerceRequest,
  type CommerceScope,
} from '@/features/commerce/native-client'

const decimalInteger = /^(0|[1-9][0-9]{0,18})$/
export const discountInteger = z
  .string()
  .refine(
    (value) =>
      decimalInteger.test(value) && BigInt(value) <= 9223372036854775807n
  )
const discountId = discountInteger
  .refine((value) => value !== '0')
  .brand<'DiscountCodeId'>()
export const discountDefinitionSchema = z.object({
  code: z.string().regex(/^[A-Z0-9][A-Z0-9_-]{2,63}$/),
  name: z.string().trim().min(1).max(120),
  discount_percent: z.number().int().min(1).max(99),
  min_credit_units: discountInteger,
  max_uses: discountInteger,
  enabled: z.boolean(),
  starts_at: discountInteger,
  ends_at: discountInteger,
})
export const nativeDiscountSchema = discountDefinitionSchema.extend({
  id: discountId,
  used_count: discountInteger,
  reserved_count: discountInteger,
})
export const discountBatchSchema = discountDefinitionSchema
  .omit({ code: true })
  .extend({
    request_key: z.string().uuid(),
    count: z.number().int().min(1).max(100),
  })
const pageSchema = z.object({
  items: z.array(nativeDiscountSchema).max(100),
  next_after: discountId.nullable(),
})
const batchResultSchema = z.object({
  items: z.array(nativeDiscountSchema).min(1).max(100),
})
const envelope = <T>(data: z.ZodType<T>) =>
  z.object({ success: z.literal(true), data })
const root = '/core/v1/payment-discounts/admin/codes'
export type NativeDiscount = z.infer<typeof nativeDiscountSchema>
export type DiscountDefinition = z.infer<typeof discountDefinitionSchema>
export type DiscountBatch = z.infer<typeof discountBatchSchema>
export type DiscountPage = z.infer<typeof pageSchema>
export type DiscountFilter = {
  keyword: string
  enabled: 'all' | 'true' | 'false'
}

export async function getDiscountPage(
  scope: CommerceScope,
  after: string,
  filter: DiscountFilter
) {
  const query = new URLSearchParams({
    after_id: discountInteger.parse(after),
    limit: '20',
  })
  if (filter.keyword.trim()) query.set('keyword', filter.keyword.trim())
  if (filter.enabled !== 'all') query.set('enabled', filter.enabled)
  return (
    await commerceRequest(
      scope,
      'get',
      `${root}?${query}`,
      envelope(pageSchema)
    )
  ).data
}
export async function saveDiscount(
  scope: CommerceScope,
  input: DiscountDefinition,
  current?: NativeDiscount
) {
  return (
    await commerceRequest(
      scope,
      current ? 'put' : 'post',
      current ? `${root}/${current.id}` : root,
      envelope(nativeDiscountSchema),
      discountDefinitionSchema.parse(input)
    )
  ).data
}
export async function generateDiscountBatch(
  scope: CommerceScope,
  input: DiscountBatch
) {
  return (
    await commerceRequest(
      scope,
      'post',
      `${root}/batch`,
      envelope(batchResultSchema),
      discountBatchSchema.parse(input)
    )
  ).data.items
}
export async function removeDiscount(
  scope: CommerceScope,
  code: NativeDiscount
) {
  await commerceRequest(
    scope,
    'delete',
    `${root}/${code.id}`,
    envelope(z.null())
  )
}
export async function removeExhaustedDiscounts(scope: CommerceScope) {
  return (
    await commerceRequest(
      scope,
      'delete',
      `${root}/exhausted`,
      envelope(z.object({ count: discountInteger }))
    )
  ).data.count
}
export async function setDiscountEnabled(
  scope: CommerceScope,
  code: NativeDiscount,
  enabled: boolean
) {
  return (
    await commerceRequest(
      scope,
      'put',
      `${root}/${code.id}/status`,
      envelope(nativeDiscountSchema),
      { enabled }
    )
  ).data
}
