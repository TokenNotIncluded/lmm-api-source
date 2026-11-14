/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/

import { api } from '@/lib/api'

import type {
  DiscountCode,
  DiscountCodeBatchInput,
  DiscountCodeInput,
  DiscountCodePage,
  DiscountCodeResponse,
} from './types.js'

const CORE_ADMIN_CODES_PREFIX = '/core/v1/payment-discounts/admin/codes'
const ENABLED_STATUS = 1
const DISABLED_STATUS = 2

interface CoreDiscountCodeView {
  id: string
  code: string
  name: string
  discount_percent: number
  min_credit_units: string
  max_uses: string
  enabled: boolean
  starts_at: string
  ends_at: string
  used_count: string
  reserved_count: string
}

interface CoreDiscountCodesPage {
  items: CoreDiscountCodeView[]
  next_after?: string | null
}

function mapCoreDiscountCode(item: CoreDiscountCodeView): DiscountCode {
  return {
    id: Number(item.id),
    code: item.code,
    name: item.name,
    discount_percent: item.discount_percent,
    min_amount: Number(item.min_credit_units),
    status: item.enabled ? ENABLED_STATUS : DISABLED_STATUS,
    used_count: Number(item.used_count),
    max_uses: Number(item.max_uses),
    created_by: 0,
    created_time: 0,
    updated_time: 0,
    starts_time: Number(item.starts_at),
    expired_time: Number(item.ends_at),
  }
}

export async function listDiscountCodes(params: {
  page: number
  pageSize: number
  keyword?: string
  status?: string
}): Promise<DiscountCodeResponse<DiscountCodePage>> {
  const targetPage = Math.max(1, params.page)
  let afterId: string | undefined
  let pageData: CoreDiscountCodesPage = { items: [], next_after: null }

  for (let current = 1; current <= targetPage; current += 1) {
    const query = new URLSearchParams({
      limit: String(params.pageSize),
    })
    if (afterId) query.set('after_id', afterId)
    if (params.keyword?.trim()) query.set('keyword', params.keyword.trim())
    if (params.status === '1' || params.status === 'true') {
      query.set('enabled', 'true')
    } else if (
      params.status === '0' ||
      params.status === '2' ||
      params.status === 'false'
    ) {
      query.set('enabled', 'false')
    }

    const response = await api.get<DiscountCodeResponse<CoreDiscountCodesPage>>(
      `${CORE_ADMIN_CODES_PREFIX}?${query.toString()}`
    )
    if (!response.data.success || !response.data.data) {
      return {
        success: response.data.success,
        message: response.data.message,
      }
    }
    pageData = response.data.data
    if (!pageData.next_after) break
    afterId = pageData.next_after
  }

  const items = pageData.items.map(mapCoreDiscountCode)
  const hasMore = Boolean(pageData.next_after)
  const total = hasMore
    ? targetPage * params.pageSize + 1
    : (targetPage - 1) * params.pageSize + items.length

  return {
    success: true,
    data: {
      items,
      total,
      page: targetPage,
      page_size: params.pageSize,
    },
  }
}

export async function createDiscountCodes(
  input: DiscountCodeBatchInput
): Promise<DiscountCodeResponse<DiscountCode[]>> {
  const response = await api.post<
    DiscountCodeResponse<{ items: CoreDiscountCodeView[] }>
  >(`${CORE_ADMIN_CODES_PREFIX}/batch`, {
    request_key: crypto.randomUUID(),
    count: input.count,
    name: input.name,
    discount_percent: input.discount_percent,
    min_credit_units: input.min_amount,
    max_uses: input.max_uses,
    enabled: true,
    starts_at: input.starts_time,
    ends_at: input.expired_time,
  })
  return {
    success: response.data.success,
    message: response.data.message,
    data: response.data.data?.items.map(mapCoreDiscountCode),
  }
}

export async function updateDiscountCode(
  input: DiscountCodeInput & { id: number }
): Promise<DiscountCodeResponse<DiscountCode>> {
  const response = await api.put<DiscountCodeResponse<CoreDiscountCodeView>>(
    `${CORE_ADMIN_CODES_PREFIX}/${input.id}`,
    {
      code: input.code,
      name: input.name,
      discount_percent: input.discount_percent,
      min_credit_units: input.min_amount,
      max_uses: input.max_uses,
      enabled:
        input.status === undefined ? true : input.status === ENABLED_STATUS,
      starts_at: input.starts_time,
      ends_at: input.expired_time,
    }
  )
  return {
    success: response.data.success,
    message: response.data.message,
    data: response.data.data
      ? mapCoreDiscountCode(response.data.data)
      : undefined,
  }
}

export async function updateDiscountCodeStatus(
  id: number,
  status: number
): Promise<DiscountCodeResponse<DiscountCode>> {
  const response = await api.put<DiscountCodeResponse<CoreDiscountCodeView>>(
    `${CORE_ADMIN_CODES_PREFIX}/${id}/status`,
    {
      enabled: status === ENABLED_STATUS,
    }
  )
  return {
    success: response.data.success,
    message: response.data.message,
    data: response.data.data
      ? mapCoreDiscountCode(response.data.data)
      : undefined,
  }
}

export async function deleteDiscountCode(
  id: number
): Promise<DiscountCodeResponse<null>> {
  const response = await api.delete(`${CORE_ADMIN_CODES_PREFIX}/${id}`)
  return response.data
}

export async function deleteExhaustedDiscountCodes(): Promise<
  DiscountCodeResponse<{ count: number }>
> {
  const response = await api.delete<
    DiscountCodeResponse<{ count: string | number }>
  >(`${CORE_ADMIN_CODES_PREFIX}/exhausted`)
  return {
    success: response.data.success,
    message: response.data.message,
    data: response.data.data
      ? { count: Number(response.data.data.count) }
      : undefined,
  }
}
