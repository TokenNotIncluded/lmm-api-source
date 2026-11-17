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
import { useMutation, useQueryClient } from '@tanstack/react-query'
import i18next from 'i18next'
import { toast } from 'sonner'

import {
  createCustomOAuthProvider,
  updateCustomOAuthProvider,
  deleteCustomOAuthProvider,
  discoverOIDCEndpoints,
} from '../api'
import type {
  CustomOAuthFormValues,
  CustomOAuthProvider,
  DiscoveryResponse,
} from '../types'

function useInvalidateOnSuccess() {
  const queryClient = useQueryClient()
  return {
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['custom-oauth-providers'] })
      queryClient.invalidateQueries({ queryKey: ['status'] })
    },
  }
}

export function useCreateProvider() {
  const invalidate = useInvalidateOnSuccess()

  return useMutation({
    mutationFn: ({
      data,
      operationKey,
    }: {
      data: CustomOAuthFormValues
      operationKey: string
    }) => createCustomOAuthProvider(data, operationKey),
    onSuccess: (res) => {
      if (res.success) {
        toast.success(i18next.t('Provider created successfully'))
        invalidate.onSuccess()
      }
    },
    onError: (error: Error) => {
      toast.error(error.message || i18next.t('Failed to create provider'))
    },
  })
}

export function useUpdateProvider() {
  const invalidate = useInvalidateOnSuccess()

  return useMutation({
    mutationFn: ({
      provider,
      data,
      operationKey,
    }: {
      provider: Pick<CustomOAuthProvider, 'id' | 'revision'>
      data: Partial<CustomOAuthFormValues>
      operationKey: string
    }) => updateCustomOAuthProvider(provider, data, operationKey),
    onSuccess: (res) => {
      if (res.success) {
        toast.success(i18next.t('Provider updated successfully'))
        invalidate.onSuccess()
      }
    },
    onError: (error: Error) => {
      toast.error(error.message || i18next.t('Failed to update provider'))
    },
  })
}

export function useDeleteProvider() {
  const invalidate = useInvalidateOnSuccess()

  return useMutation({
    mutationFn: ({
      provider,
      operationKey,
    }: {
      provider: Pick<CustomOAuthProvider, 'id' | 'revision'>
      operationKey: string
    }) => deleteCustomOAuthProvider(provider, operationKey),
    onSuccess: (res) => {
      if (res.success) {
        toast.success(i18next.t('Provider deleted successfully'))
        invalidate.onSuccess()
      }
    },
    onError: (error: Error) => {
      toast.error(error.message || i18next.t('Failed to delete provider'))
    },
  })
}

export function useDiscoverEndpoints() {
  return useMutation({
    mutationFn: (wellKnownUrl: string) => discoverOIDCEndpoints(wellKnownUrl),
    onSuccess: (res: DiscoveryResponse) => {
      if (res.success) {
        toast.success(i18next.t('OIDC endpoints discovered successfully'))
      }
    },
    onError: (error: Error) => {
      toast.error(
        error.message || i18next.t('Failed to discover OIDC endpoints')
      )
    },
  })
}
