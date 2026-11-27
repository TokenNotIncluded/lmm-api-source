/* Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later. */
import { api } from '@/lib/api'

import { getPublicDirectory } from './public-api'

export type DirectoryAd = {
  id: number
  name: string
  url: string
  summary: string
  description: string
  bid_cents: number
  charged_quota: number
  charged_amount_usd: string | null
  status: 'active' | 'hidden'
  paid_at: number
  expires_at: number
  hidden_at: number
  refunded_at: number
}

export type DirectoryAdQuote = {
  pricing_schema_version: 2
  bid_cents: number
  quota: number
  currency: 'USD'
  duration_days: number
  min_bid_cents: number
  max_bid_cents: number
  quote_id: string
  request_id: string
  content_sha256: string
  quote_contract_version: number
  expires_at: number
  confirmation_path: string
}

export function isUsdDirectoryAdQuote(
  value: unknown
): value is DirectoryAdQuote {
  if (!value || typeof value !== 'object') return false
  const quote = value as Partial<DirectoryAdQuote>
  return (
    quote.pricing_schema_version === 2 &&
    quote.currency === 'USD' &&
    Number.isSafeInteger(quote.quota) &&
    Number(quote.quota) > 0 &&
    Number.isSafeInteger(quote.bid_cents) &&
    Number(quote.bid_cents) >= 100
  )
}

export type DirectoryAdDraft = {
  name: string
  url: string
  summary: string
  description: string
  bid_cents: number
  request_id: string
}

export type DirectoryAdInput = DirectoryAdDraft & {
  expected_quota: number
  quote_id: string
  content_sha256: string
}

type ApiEnvelope<T> = {
  success: boolean
  code?: string
  message?: string
  data: T
}

async function unwrap<T>(
  request: Promise<{ data: ApiEnvelope<T> }>
): Promise<T> {
  const response = await request
  if (!response.data.success) {
    const error = new Error(
      response.data.message || 'Unable to load advertisements'
    )
    Object.assign(error, { code: response.data.code })
    throw error
  }
  return response.data.data
}

export function listDirectoryAds(offset = 0) {
  return unwrap<{
    items: DirectoryAd[]
    has_more: boolean
    next_offset: number
  }>(getPublicDirectory(`/api/ai-directory/ads?offset=${offset}`))
}

export async function prepareDirectoryAd(input: DirectoryAdDraft) {
  const quote = await unwrap<DirectoryAdQuote>(
    api.post('/api/ai-directory/ads/prepare', input, {
      skipErrorHandler: true,
      skipBusinessError: true,
    })
  )
  if (
    !isUsdDirectoryAdQuote(quote) ||
    quote.request_id !== input.request_id ||
    quote.bid_cents !== input.bid_cents ||
    quote.quote_contract_version !== 1 ||
    !/^[a-f0-9]{64}$/.test(quote.quote_id) ||
    !/^[a-f0-9]{64}$/.test(quote.content_sha256) ||
    !Number.isSafeInteger(quote.expires_at) ||
    quote.expires_at <= Date.now() / 1000 ||
    quote.confirmation_path !==
      `/core/v1/directory-ads/confirm/${quote.quote_id}`
  ) {
    throw new Error('Unable to get a price quote. Try again.')
  }
  return quote
}

export function createDirectoryAd(input: DirectoryAdInput) {
  return unwrap<{ ad: DirectoryAd; created: boolean; charged_quota: number }>(
    api.post('/api/ai-directory/ads', input, {
      skipErrorHandler: true,
      skipBusinessError: true,
    })
  )
}

export function listMyDirectoryAds() {
  return unwrap<{ items: DirectoryAd[] }>(
    api.get('/api/ai-directory/ads/mine', {
      skipErrorHandler: true,
      skipBusinessError: true,
    })
  )
}

export async function hideDirectoryAd(
  id: number
): Promise<'refunded' | 'pending'> {
  const result = await unwrap<{
    ad: DirectoryAd
    refunded: boolean
    refunded_quota: number
    refund_pending: boolean
  }>(
    api.post(`/api/ai-directory/ads/${id}/hide`, null, {
      skipErrorHandler: true,
      skipBusinessError: true,
    })
  )
  if (result?.ad?.id === id && result.ad.status === 'hidden') {
    if (
      result.refunded === true &&
      result.refund_pending === false &&
      Number.isSafeInteger(result.refunded_quota) &&
      result.refunded_quota > 0 &&
      result.refunded_quota === result.ad.charged_quota
    )
      return 'refunded'
    if (
      result.refunded === false &&
      result.refund_pending === true &&
      result.refunded_quota === 0
    ) {
      return 'pending'
    }
  }
  throw new Error('Invalid advertisement refund receipt')
}

export function directoryAdErrorCode(error: unknown): string | null {
  if (!error || typeof error !== 'object') return null
  const response = (error as { response?: { data?: { code?: unknown } } })
    .response
  if (typeof response?.data?.code === 'string') return response.data.code
  const direct = (error as { code?: unknown }).code
  return typeof direct === 'string' ? direct : null
}
