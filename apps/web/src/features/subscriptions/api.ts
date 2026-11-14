/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.

For commercial licensing, please contact support@quantumnous.com
*/
import { api } from '@/lib/api'
import type { WaffoPancakeCheckoutLanguage } from '@/lib/waffo-pancake-checkout'

import type {
  ApiResponse,
  PlanRecord,
  UserSubscriptionRecord,
  AdminSubscriptionResetEligiblePage,
  SubscriptionResetPreviewRequest,
  SubscriptionResetPreviewResult,
  SubscriptionResetExecuteRequest,
  SubscriptionResetBatchResult,
  SubscriptionResetVoucher,
  SubscriptionResetResult,
  SubscriptionPayResponse,
  SubscriptionPayRequest,
  WaffoPancakeSubscriptionPayRequest,
  SelfSubscriptionData,
} from './types'

// ============================================================================
// Admin Plan Management
// ============================================================================

export async function getAdminPlans(
  includeArchived = false
): Promise<ApiResponse<PlanRecord[]>> {
  const res = await api.get('/api/subscription/admin/plans', {
    params: includeArchived ? { include_archived: '1' } : undefined,
  })
  return res.data
}

export async function getSubscriptionResetEligible(
  params: {
    page: number
    pageSize: number
    query?: string
    planIds?: number[]
    userIds?: number[]
  },
  signal?: AbortSignal
): Promise<ApiResponse<AdminSubscriptionResetEligiblePage>> {
  const res = await api.get('/api/subscription/root/reset-targets', {
    params: {
      page: params.page,
      page_size: params.pageSize,
      query: params.query || undefined,
      plan_ids: params.planIds?.join(',') || undefined,
      user_ids: params.userIds?.join(',') || undefined,
    },
    signal,
  })
  return res.data
}

export async function previewSubscriptionReset(
  data: SubscriptionResetPreviewRequest
): Promise<ApiResponse<SubscriptionResetPreviewResult>> {
  const res = await api.post('/api/subscription/root/reset/preview', data)
  return res.data
}

export async function executeSubscriptionReset(
  data: SubscriptionResetExecuteRequest
): Promise<ApiResponse<SubscriptionResetBatchResult>> {
  const res = await api.post('/api/subscription/root/reset', data)
  return res.data
}

// ============================================================================
// User-facing Subscription Payment
// ============================================================================

export async function paySubscriptionStripe(
  data: SubscriptionPayRequest
): Promise<SubscriptionPayResponse> {
  const res = await api.post('/api/subscription/stripe/pay', data)
  return res.data
}

export async function paySubscriptionCreem(
  data: SubscriptionPayRequest
): Promise<SubscriptionPayResponse> {
  const res = await api.post('/api/subscription/creem/pay', data)
  return res.data
}

export async function paySubscriptionWaffoPancake(
  data: WaffoPancakeSubscriptionPayRequest
): Promise<SubscriptionPayResponse> {
  const res = await api.post('/api/subscription/waffo-pancake/pay', data, {
    skipBusinessError: true,
    skipErrorHandler: true,
  })
  return res.data
}

export async function paySubscriptionBalance(
  data: SubscriptionPayRequest
): Promise<SubscriptionPayResponse> {
  const res = await api.post('/api/subscription/balance/pay', data)
  return res.data
}

export async function paySubscriptionEpay(
  data: SubscriptionPayRequest & { payment_method: string }
): Promise<SubscriptionPayResponse & { url?: string }> {
  const res = await api.post('/api/subscription/epay/pay', data)
  return res.data
}

// ============================================================================
// User Self Subscriptions
// ============================================================================

export async function getSelfSubscriptions(): Promise<
  ApiResponse<UserSubscriptionRecord[]>
> {
  const res = await api.get('/api/subscription/self')
  return res.data
}

export async function getSelfSubscriptionFull(): Promise<
  ApiResponse<SelfSubscriptionData>
> {
  const res = await api.get('/api/subscription/self')
  return res.data
}

export async function getSubscriptionResetVouchers(): Promise<
  ApiResponse<SubscriptionResetVoucher[]>
> {
  const res = await api.get('/api/subscription/self/reset-vouchers')
  return res.data
}

export async function redeemSubscriptionResetVoucher(
  voucherId: number
): Promise<ApiResponse<SubscriptionResetResult>> {
  const res = await api.post(
    `/api/subscription/self/reset-vouchers/${voucherId}/redeem`
  )
  return res.data
}

// The quote must use the same locale hint as the later checkout request.
// @mark payments.settlement-locale role=transport
export async function getPublicPlans(
  signal: AbortSignal | undefined,
  checkoutLanguage: WaffoPancakeCheckoutLanguage
): Promise<ApiResponse<PlanRecord[]>> {
  const res = await api.get('/api/subscription/plans', {
    signal,
    headers: { 'Accept-Language': checkoutLanguage },
  })
  return res.data
}

export async function updateBillingPreference(
  preference: string
): Promise<ApiResponse<{ billing_preference?: string }>> {
  const res = await api.put('/api/subscription/self/preference', {
    billing_preference: preference,
  })
  return res.data
}

export async function getGroups(): Promise<ApiResponse<string[]>> {
  const res = await api.get('/api/group')
  return res.data
}
