/* Copyright (C) 2026 LIghtJUNction; SPDX-License-Identifier: AGPL-3.0-or-later */
export type ExtoreSource = {
  connection_id: string
  source_id: string
  product_id: string
  last_synced_at: number
  card_summary?: { total?: number; states?: Record<string, number> }
}

export type ExtoreConnection = {
  id: string
  origin?: string
  shop_name: string
  expires_at: number
  updated_at?: number
  status?: 'connected' | 'expired' | 'reauthorization_required'
}

export type ExtoreConnections = {
  connections: ExtoreConnection[]
  sources: ExtoreSource[]
}

export type ExtoreSyncResult = {
  source_id: string
  product_id?: string
  success: boolean
  message: string
  last_synced_at?: number
}

export type ExtoreImportResult = {
  connection: { id: string }
  results: ExtoreSyncResult[]
}

export type ExtoreBatchResult = {
  connection_id: string
  results: ExtoreSyncResult[]
}
