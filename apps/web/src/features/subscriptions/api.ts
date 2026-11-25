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
import { isAxiosError } from 'axios'

import {
  CommerceFailure,
  commerceScope,
} from '@/features/commerce/native-client'
import { api } from '@/lib/api'
import type { WaffoPancakeCheckoutLanguage } from '@/lib/waffo-pancake-checkout'
import { useAuthStore } from '@/stores/auth-store'

import {
  nativePlanToRecord,
  nativePurchasesToSelfData,
} from './lib/native-classic-bridge'
import { listNativePlans, listNativePurchases } from './native-api'
import type {
  ApiResponse,
  PlanRecord,
  UserSubscriptionRecord,
  SubscriptionPayResponse,
  SubscriptionPayRequest,
  WaffoPancakeSubscriptionPayRequest,
  SelfSubscriptionData,
} from './types'

function isClassicSubscriptionPayload<T>(
  data: unknown
): data is ApiResponse<T> {
  return (
    !!data &&
    typeof data === 'object' &&
    'success' in data &&
    typeof (data as ApiResponse<T>).success === 'boolean'
  )
}

function subscriptionOwner() {
  const { user, session } = useAuthStore.getState().auth
  return { userId: user?.id, sessionId: session?.sid }
}

function requireSubscriptionOwner(owner: ReturnType<typeof subscriptionOwner>) {
  const current = subscriptionOwner()
  if (
    current.userId !== owner.userId ||
    current.sessionId !== owner.sessionId
  ) {
    throw new CommerceFailure('SESSION_CHANGED')
  }
}

// A missing classic read selects the native catalog. Scoped read errors are
// handled by their page so this expected 404 cannot toast before the fallback;
// non-404 failures still reject without switching financial protocols.
function isMissingSubscriptionRoute(error: unknown) {
  return isAxiosError(error) && error.response?.status === 404
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
  const owner = subscriptionOwner()
  const scope = commerceScope()
  // A failed classic payment may already have committed. Native plans use the
  // native wallet's persisted quote and explicit confirmation instead.
  const res = await api.post('/api/subscription/balance/pay', data, {
    ...(scope ? { authScope: scope } : {}),
    skipAuthRefresh: true,
  })
  requireSubscriptionOwner(owner)
  if (
    !isClassicSubscriptionPayload<SubscriptionPayResponse['data']>(res.data)
  ) {
    throw new CommerceFailure('INVALID_RESPONSE')
  }
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
  const full = await getSelfSubscriptionFull()
  if (!full.success || !full.data) {
    return { success: false, message: full.message }
  }
  return { success: true, data: full.data.subscriptions }
}

export async function getSelfSubscriptionFull(): Promise<
  ApiResponse<SelfSubscriptionData>
> {
  const owner = subscriptionOwner()
  const scope = commerceScope()
  try {
    const res = await api.get(
      '/api/subscription/self',
      scope
        ? { authScope: scope, skipAuthRefresh: true, skipErrorHandler: true }
        : undefined
    )
    requireSubscriptionOwner(owner)
    if (!isClassicSubscriptionPayload<SelfSubscriptionData>(res.data)) {
      throw new CommerceFailure('INVALID_RESPONSE')
    }
    return res.data
  } catch (error) {
    requireSubscriptionOwner(owner)
    if (!scope || !isMissingSubscriptionRoute(error)) throw error
  }
  const purchases = await listNativePurchases(scope)
  requireSubscriptionOwner(owner)
  return {
    success: true,
    data: nativePurchasesToSelfData(purchases, scope.userId),
  }
}

// The quote must use the same locale hint as the later checkout request.
// @mark payments.settlement-locale role=transport
export async function getPublicPlans(
  signal: AbortSignal | undefined,
  checkoutLanguage: WaffoPancakeCheckoutLanguage
): Promise<ApiResponse<PlanRecord[]>> {
  const owner = subscriptionOwner()
  const scope = commerceScope()
  try {
    const res = await api.get('/api/subscription/plans', {
      signal,
      headers: { 'Accept-Language': checkoutLanguage },
      ...(scope
        ? { authScope: scope, skipAuthRefresh: true, skipErrorHandler: true }
        : {}),
    })
    requireSubscriptionOwner(owner)
    signal?.throwIfAborted()
    if (
      !isClassicSubscriptionPayload<PlanRecord[]>(res.data) ||
      (res.data.success && !Array.isArray(res.data.data))
    ) {
      throw new CommerceFailure('INVALID_RESPONSE')
    }
    return res.data
  } catch (error) {
    requireSubscriptionOwner(owner)
    signal?.throwIfAborted()
    if (!scope || !isMissingSubscriptionRoute(error)) throw error
  }
  const plans = await listNativePlans(scope)
  requireSubscriptionOwner(owner)
  signal?.throwIfAborted()
  return { success: true, data: plans.map(nativePlanToRecord) }
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
