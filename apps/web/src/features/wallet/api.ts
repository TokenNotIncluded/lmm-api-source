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
import { useAuthStore } from '@/stores/auth-store'

import { bindTopupOrder, capturePreparedTopup } from './lib/topup-cloud-storage'
import { hasCompleteCreditGrant } from './lib/topup-credit-metadata'
import type {
  CreditAmountRequest,
  CreditPaymentRequest,
  CreditWaffoPaymentRequest,
  CreditPancakePaymentRequest,
  CreditWireRequestUnit,
  VersionedCreditAmountRequest,
  VersionedCreditPaymentRequest,
  VersionedCreditWaffoPaymentRequest,
  VersionedCreditPancakePaymentRequest,
  ApiResponse,
  TopupInfoResponse,
  AmountResponse,
  PaymentResponse,
  StripePaymentResponse,
  AffiliateCodeResponse,
  BillingHistoryResponse,
  BillingHistorySortBy,
  BillingHistorySortOrder,
  CompleteOrderRequest,
  WaffoPaymentResponse,
  WaffoPancakePaymentResponse,
  DiscountCodeResponse,
} from './types'

// Capture the initiating account/intent before awaiting and bind the server order
// before a payment hook can redirect the browser away from this document.
async function checkoutRequest<T>(request: () => Promise<T>): Promise<T> {
  const auth = useAuthStore.getState().auth
  const intent = capturePreparedTopup(auth.user?.id)
  const result = await request()
  const current = useAuthStore.getState().auth
  if (
    current.user?.id === auth.user?.id &&
    current.session?.sid === auth.session?.sid
  ) {
    bindTopupOrder(intent, result)
  }
  return result
}

// ============================================================================
// Wallet API Functions
// ============================================================================

/**
 * Check if API response is successful
 */
export function isApiSuccess(response: ApiResponse): boolean {
  return response.success === true
}

/**
 * Get topup configuration info
 */
export async function getTopupInfo(
  signal?: AbortSignal
): Promise<TopupInfoResponse> {
  // Payment availability is optional onboarding decoration.  The caller
  // renders an inline fallback when this probe is unavailable, so an
  // inactive/legacy listener must not turn a harmless 401/404 into a global
  // toast (or a duplicate error on every focus refresh).
  const res = await api.get('/api/user/topup/info', {
    signal,
    skipBusinessError: true,
    skipErrorHandler: true,
  })
  return res.data
}

/**
 * Get affiliate code
 */
export async function getAffiliateCode(): Promise<AffiliateCodeResponse> {
  const res = await api.get('/core/v1/affiliate/code')
  return res.data
}

/** Prepare a session-bound transfer; the caller must show it before confirmation. */
export async function previewAffiliateTransfer(amountCredits: string): Promise<{
  id: number
  amount: number
  csrf: string
}> {
  if (
    !/^[1-9]\d*$/.test(amountCredits) ||
    !Number.isSafeInteger(Number(amountCredits))
  ) {
    throw new Error('Invalid affiliate transfer amount')
  }
  const res = await api.post('/core/v1/affiliate/preview', {
    amount_credits: amountCredits,
  })
  return res.data
}

/** Retry the same preview ID and token to recover the same durable receipt. */
export async function confirmAffiliateTransfer(
  id: number,
  csrf: string
): Promise<{
  id: number
  amount_credits: number
  outcome: { status: 'posted' | 'rejected' }
}> {
  const res = await api.post(`/core/v1/affiliate/claim/${id}`, { csrf })
  return res.data
}

/**
 * Get billing history for current user
 */
export async function getUserBillingHistory(
  page: number,
  pageSize: number,
  keyword?: string,
  sortBy: BillingHistorySortBy = 'create_time',
  sortOrder: BillingHistorySortOrder = 'desc'
): Promise<ApiResponse<BillingHistoryResponse>> {
  const params = new URLSearchParams({
    p: page.toString(),
    page_size: pageSize.toString(),
    sort_by: sortBy,
    sort_order: sortOrder,
  })
  if (keyword) {
    params.append('keyword', keyword)
  }
  const res = await api.get(`/api/user/topup/self?${params.toString()}`)
  return res.data
}

/**
 * Get billing history for all users (admin only)
 */
export async function getAllBillingHistory(
  page: number,
  pageSize: number,
  keyword?: string,
  sortBy: BillingHistorySortBy = 'create_time',
  sortOrder: BillingHistorySortOrder = 'desc'
): Promise<ApiResponse<BillingHistoryResponse>> {
  const params = new URLSearchParams({
    p: page.toString(),
    page_size: pageSize.toString(),
    sort_by: sortBy,
    sort_order: sortOrder,
  })
  if (keyword) {
    params.append('keyword', keyword)
  }
  const res = await api.get(`/api/user/topup?${params.toString()}`)
  return res.data
}

/**
 * Complete a pending order (admin only)
 */
export async function completeOrder(
  request: CompleteOrderRequest
): Promise<ApiResponse> {
  const res = await api.post('/api/user/topup/complete', request)
  return res.data
}

/**
 * Historical Credit* callers supply raw ledger integers. Only explicit v2
 * CREDIT requests use the public denomination; old nodes must never receive a fallback.
 */
async function creditMoneyRequest<T>(
  endpoint: string,
  request: { amount: number } & CreditWireRequestUnit
): Promise<T> {
  if (!Number.isSafeInteger(request.amount) || request.amount <= 0) {
    throw new Error('Invalid top-up amount')
  }
  let unit: 'LEDGER_QUOTA' | 'CREDIT' = 'LEDGER_QUOTA'
  const version = request.credit_metadata_version
  const expected = request.expected_public_credits_per_usd_exact
  if (version === undefined) {
    if (
      (request.amount_unit !== undefined && request.amount_unit !== 'CREDIT') ||
      expected !== undefined
    ) {
      throw new Error('Invalid top-up credit metadata')
    }
  } else if (
    version === 2 &&
    request.amount_unit === 'LEDGER_QUOTA' &&
    expected === undefined
  ) {
    unit = 'LEDGER_QUOTA'
  } else if (
    version === 2 &&
    request.amount_unit === 'CREDIT' &&
    typeof expected === 'string' &&
    /^[1-9]\d{0,15}$/.test(expected) &&
    Number.isSafeInteger(Number(expected))
  ) {
    unit = 'CREDIT'
  } else {
    throw new Error('Invalid top-up credit metadata')
  }
  const response = await api.post(
    `/api/user/topup/currency/v2/${endpoint}`,
    {
      ...request,
      amount_unit: unit,
      credit_metadata_version: 2,
    },
    { skipBusinessError: true } as Record<string, unknown>
  )
  const result = response.data
  if (
    endpoint !== 'discount-code/validate' &&
    result &&
    typeof result === 'object' &&
    isApiSuccess(result)
  ) {
    const grant =
      'credit_amount_unit' in result ||
      'public_credit_metadata_version' in result
        ? result
        : result.data
    if (
      !hasCompleteCreditGrant(
        grant,
        unit === 'LEDGER_QUOTA' ? request.amount : undefined,
        unit === 'CREDIT' ? request.amount : undefined
      ) ||
      (unit === 'CREDIT' && grant.public_credits_per_usd_exact !== expected)
    ) {
      throw new Error('Top-up credit metadata unavailable')
    }
  }
  return response.data
}

export const calculateCreditAmount = (
  request: CreditAmountRequest | VersionedCreditAmountRequest
) => creditMoneyRequest<AmountResponse>('amount', request)
export const calculateCreditStripeAmount = (
  request: CreditAmountRequest | VersionedCreditAmountRequest
) => creditMoneyRequest<AmountResponse>('stripe/amount', request)
export const calculateCreditWaffoAmount = (
  request: CreditAmountRequest | VersionedCreditAmountRequest
) => creditMoneyRequest<AmountResponse>('waffo/amount', request)
export const calculateCreditPancakeAmount = (
  request: CreditAmountRequest | VersionedCreditAmountRequest
) => creditMoneyRequest<AmountResponse>('waffo-pancake/amount', request)
export const validateCreditDiscountCode = (
  request: {
    code: string
    amount: number
    payment_method?: string
  } & CreditWireRequestUnit
) => creditMoneyRequest<DiscountCodeResponse>('discount-code/validate', request)
export const requestCreditPayment = (
  request: CreditPaymentRequest | VersionedCreditPaymentRequest
) => checkoutRequest(() => creditMoneyRequest<PaymentResponse>('pay', request))
export const requestCreditStripePayment = (
  request: CreditPaymentRequest | VersionedCreditPaymentRequest
) =>
  checkoutRequest(() =>
    creditMoneyRequest<StripePaymentResponse>('stripe/pay', request)
  )
export const requestCreditWaffoPayment = (
  request: CreditWaffoPaymentRequest | VersionedCreditWaffoPaymentRequest
) =>
  checkoutRequest(() =>
    creditMoneyRequest<WaffoPaymentResponse>('waffo/pay', request)
  )
export const requestCreditPancakePayment = (
  request: CreditPancakePaymentRequest | VersionedCreditPancakePaymentRequest
) =>
  checkoutRequest(() =>
    creditMoneyRequest<WaffoPancakePaymentResponse>(
      'waffo-pancake/pay',
      request
    )
  )
