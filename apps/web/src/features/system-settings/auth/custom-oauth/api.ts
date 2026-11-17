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
import axios from 'axios'
import i18next from 'i18next'
import * as z from 'zod'

import { api } from '@/lib/api'

import {
  customOAuthFormSchema,
  customOAuthProviderSchema,
  type CustomOAuthFormValues,
  type CustomOAuthProvider,
  type DiscoveryResponse,
} from './types'

// ============================================================================
// Response Types
// ============================================================================

interface ApiResponse<T> {
  success: true
  message?: string
  data: T
}

class ProviderResponseError extends Error {
  constructor(
    message: string,
    readonly resultUnknown: boolean
  ) {
    super(message)
  }
}

export function isProviderWriteResultUnknown(error: unknown): boolean {
  if (error instanceof ProviderResponseError) return error.resultUnknown
  return (
    !axios.isAxiosError(error) ||
    !error.response ||
    error.response.status >= 500
  )
}

function providerResponse<T>(
  payload: { success?: boolean; message?: string; data?: unknown },
  schema: z.ZodType<T>,
  fallback: string
): ApiResponse<T> {
  if (payload?.success !== true) {
    throw new ProviderResponseError(
      payload?.message || fallback,
      payload?.success !== false
    )
  }
  const parsed = schema.safeParse(payload.data)
  if (!parsed.success) throw new ProviderResponseError(fallback, true)
  return { success: true, message: payload.message, data: parsed.data }
}

const writeOptions = {
  // A provider write is tied to its operation key and the current Session.
  // Recovery is an explicit retry of that same request, never an auth replay.
  skipAuthRefresh: true,
  skipBusinessError: true,
  skipErrorHandler: true,
}
const writableFields = new Set(Object.keys(customOAuthFormSchema.shape))
function providerChanges(data: Partial<CustomOAuthFormValues>) {
  return Object.fromEntries(
    Object.entries(data).filter(([key]) => writableFields.has(key))
  )
}

// ============================================================================
// Custom OAuth Provider APIs
// ============================================================================

export async function getCustomOAuthProviders(): Promise<
  ApiResponse<CustomOAuthProvider[]>
> {
  const res = await api.get('/api/custom-oauth-provider/')
  return providerResponse(
    res.data,
    z.array(customOAuthProviderSchema),
    i18next.t('Failed to load settings')
  )
}

export async function getCustomOAuthProvider(
  id: string
): Promise<ApiResponse<CustomOAuthProvider>> {
  const res = await api.get(`/api/custom-oauth-provider/${id}`)
  return providerResponse(
    res.data,
    customOAuthProviderSchema,
    i18next.t('Failed to load settings')
  )
}

export async function createCustomOAuthProvider(
  data: CustomOAuthFormValues,
  operationKey: string
): Promise<ApiResponse<CustomOAuthProvider>> {
  const res = await api.post(
    '/api/custom-oauth-provider/',
    { ...providerChanges(data), operation_key: operationKey },
    writeOptions
  )
  return providerResponse(
    res.data,
    customOAuthProviderSchema,
    i18next.t('Failed to create provider')
  )
}

export async function updateCustomOAuthProvider(
  provider: Pick<CustomOAuthProvider, 'id' | 'revision'>,
  data: Partial<CustomOAuthFormValues>,
  operationKey: string
): Promise<ApiResponse<CustomOAuthProvider>> {
  const res = await api.put(
    `/api/custom-oauth-provider/${provider.id}`,
    {
      ...providerChanges(data),
      expected_revision: provider.revision,
      operation_key: operationKey,
    },
    writeOptions
  )
  return providerResponse(
    res.data,
    customOAuthProviderSchema,
    i18next.t('Failed to update provider')
  )
}

export async function deleteCustomOAuthProvider(
  provider: Pick<CustomOAuthProvider, 'id' | 'revision'>,
  operationKey: string
): Promise<ApiResponse<CustomOAuthProvider>> {
  const res = await api.delete(`/api/custom-oauth-provider/${provider.id}`, {
    data: {
      expected_revision: provider.revision,
      operation_key: operationKey,
    },
    ...writeOptions,
  })
  return providerResponse(
    res.data,
    customOAuthProviderSchema,
    i18next.t('Failed to delete provider')
  )
}

export async function discoverOIDCEndpoints(
  wellKnownUrl: string
): Promise<DiscoveryResponse> {
  const res = await api.post('/api/custom-oauth-provider/discovery', {
    well_known_url: wellKnownUrl,
  })
  return res.data
}
