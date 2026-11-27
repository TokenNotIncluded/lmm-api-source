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
/*
Copyright (C) 2026 LIghtJUNction
*/
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

import { useQueryClient } from '@tanstack/react-query'
import {
  createContext,
  useCallback,
  useContext,
  useRef,
  useState,
  type Dispatch,
  type ReactNode,
  type SetStateAction,
} from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { refreshCurrentAccount } from '@/features/onboarding/use-auth-user-refresh'
import { useDialogState } from '@/hooks/use-dialog'
import { formatApiKey } from '@/lib/api-key-format'

import { fetchTokenKeysBatch } from '../api'
import { ERROR_MESSAGES } from '../constants'
import type { ApiKey, ApiKeysDialogType } from '../types'

type ApiKeysContextType = {
  open: ApiKeysDialogType | null
  setOpen: (str: ApiKeysDialogType | null) => void
  currentRow: ApiKey | null
  setCurrentRow: Dispatch<SetStateAction<ApiKey | null>>
  refreshTrigger: number
  triggerRefresh: () => void
  resolveRealKeysBatch: (ids: number[]) => Promise<Record<number, string>>
}

const ApiKeysContext = createContext<ApiKeysContextType | null>(null)

export function ApiKeysProvider({ children }: { children: ReactNode }) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const secretGeneration = useRef(0)
  const [open, setOpen] = useDialogState<ApiKeysDialogType>(null)
  const [currentRow, setCurrentRow] = useState<ApiKey | null>(null)
  const [refreshTrigger, setRefreshTrigger] = useState(0)
  const triggerRefresh = useCallback(() => {
    secretGeneration.current += 1
    setRefreshTrigger((prev) => prev + 1)
    void refreshCurrentAccount()
    void Promise.all(
      [
        'keys',
        'api-key',
        'user-models',
        'user-models-ccswitch',
        'user-groups',
        'pricing',
        'assistant-pricing',
        'assistant-available-models',
        'assistant-status',
        'assistant-journey',
        'assistant-onboarding-todo',
      ].map((key) => queryClient.invalidateQueries({ queryKey: [key] }))
    ).catch(() => undefined)
  }, [queryClient])

  const resolveRealKeysBatch = useCallback(
    async (ids: number[]): Promise<Record<number, string>> => {
      const generation = secretGeneration.current
      try {
        const res = await fetchTokenKeysBatch(ids)
        if (generation !== secretGeneration.current) return {}
        if (res.success && res.data?.keys) {
          return Object.fromEntries(
            Object.entries(res.data.keys).map(([id, key]) => [
              id,
              formatApiKey(key),
            ])
          )
        }
        toast.error(res.message || t(ERROR_MESSAGES.UNEXPECTED))
      } catch {
        toast.error(t(ERROR_MESSAGES.UNEXPECTED))
      }
      return {}
    },
    [t]
  )

  return (
    <ApiKeysContext
      value={{
        open,
        setOpen,
        currentRow,
        setCurrentRow,
        refreshTrigger,
        triggerRefresh,
        resolveRealKeysBatch,
      }}
    >
      {children}
    </ApiKeysContext>
  )
}

// eslint-disable-next-line react-refresh/only-export-components
export const useApiKeys = () => {
  const apiKeysContext = useContext(ApiKeysContext)

  if (!apiKeysContext) {
    throw new Error('useApiKeys has to be used within <ApiKeysContext>')
  }

  return apiKeysContext
}
