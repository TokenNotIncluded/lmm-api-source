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
import { useMemo } from 'react'

import type { SystemStatus } from '@/features/auth/types'
import { useStatus } from '@/hooks/use-status'

import {
  type ChatPreset,
  parseChatConfig,
  type RawChatConfig,
} from '../lib/chat-links'

function extractServerAddress(status: SystemStatus | null) {
  const fromStatus = status?.server_address

  if (fromStatus && typeof fromStatus === 'string') {
    return fromStatus
  }

  if (typeof window !== 'undefined') {
    return window.location.origin
  }

  return ''
}

function extractChats(status: SystemStatus | null): RawChatConfig {
  return status?.Chats as RawChatConfig
}

export function useChatPresets(): {
  chatPresets: ChatPreset[]
  serverAddress: string
  loading: boolean
  error: Error | null
  retry: () => Promise<unknown>
} {
  const { status, loading, error, refetch } = useStatus()

  const serverAddress = useMemo(() => extractServerAddress(status), [status])

  const chatPresets = useMemo(() => {
    const raw = extractChats(status)
    return parseChatConfig(raw)
  }, [status])

  return {
    chatPresets,
    serverAddress,
    loading,
    error:
      error instanceof Error ? error : error ? new Error(String(error)) : null,
    retry: () => refetch(),
  }
}
