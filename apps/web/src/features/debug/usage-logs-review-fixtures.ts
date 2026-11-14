/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import type { AxiosAdapter } from 'axios'

import { useAuthStore } from '@/stores/auth-store'

const now = Math.floor(Date.now() / 1000)

/** Synthetic records for the loopback-only console review entry. */
function records(administrator: boolean) {
  const userId = useAuthStore.getState().auth.user?.id ?? 1
  const samples = [
    ['deepseek-chat', 'success', 12500, 1800, 420, 1834.42, 186.12, true],
    ['claude-sonnet-4-5', 'success', 4800, 720, 180, 962.18, 242.04, true],
    ['gpt-4.1-mini', 'error', 0, 0, 0, 309.72, null, false],
    ['deepseek-chat', 'pending', null, null, null, null, null, null],
    ['claude-sonnet-4-5', 'refunded', 0, 800, 240, 1428.09, 225.5, true],
    ['gpt-4.1-mini', 'success', 2100, 600, 100, null, null, null],
  ] as const
  return samples.map(
    ([model, status, quota, input, output, duration, first, stream], index) => {
      const pending = status === 'pending'
      const refunded = status === 'refunded' ? 6500 : 0
      const gross = typeof quota === 'number' ? quota + refunded : null
      const state = pending ? 'reconcile' : 'settled'
      const priceVersion = 'preview-price-v1'
      const statusCode =
        duration === null ? null : status === 'error' ? 429 : 200
      const other = {
        billing_source: 'wallet',
        billing_basis: pending ? 'incomplete' : 'net_consumption',
        request_state: state,
        pricing_credits_per_usd: 500000,
        gross_settled_credits: gross,
        refunded_credits: pending ? null : refunded,
        net_consumption_credits: quota,
        cache_tokens: pending ? null : index === 0 ? 1200 : 0,
        cache_creation_tokens: index === 1 ? 96 : null,
        cache_creation_tokens_5m: index === 1 ? 96 : null,
        cache_creation_tokens_1h: index === 1 ? 0 : null,
        cache_creation_tokens_12h: index === 1 ? 0 : null,
        reserved_credits: pending ? 5000 : 0,
        token_usage_available: !pending,
        pricing_dimension: 'text',
        payer_kind: 'personal',
        wallet_debit_supported: false,
        price_version: priceVersion,
        duration_ms: duration,
        frt: first,
        status_code: statusCode,
      }
      return {
        id: 1001 + index,
        user_id: userId,
        created_at: now - 120 - index * 61,
        type: status === 'error' || pending ? 5 : 2,
        content:
          status === 'error'
            ? 'upstream_rate_limited'
            : pending
              ? 'pending'
              : 'completed',
        request_id: `demo-request-${1001 + index}`,
        upstream_request_id:
          duration === null ? null : `upstream-${1001 + index}`,
        model_name: model,
        token_name: index % 2 ? 'research-notebook' : 'application-key',
        group: 'default',
        state,
        outcome: pending ? null : status === 'error' ? 'failed' : 'completed',
        status,
        quota,
        quota_usd:
          typeof quota === 'number' ? (quota / 500000).toFixed(6) : undefined,
        quota_money_available: !pending,
        quota_money_basis: 'captured_credits_per_usd',
        billing_basis: other.billing_basis,
        gross_settled_credits: gross,
        refunded_credits: pending ? null : refunded,
        net_consumption_credits: quota,
        prompt_tokens: input,
        completion_tokens: output,
        duration_ms: duration,
        use_time: duration === null ? null : duration / 1000,
        first_token_ms: first,
        diagnostics_available: duration !== null,
        is_stream: stream,
        status_code: statusCode,
        price_version: priceVersion,
        token_usage_available: !pending,
        pricing_dimension: 'text',
        payer_kind: 'personal',
        wallet_debit_supported: false,
        ...(administrator
          ? {
              username: 'preview-user',
              channel: index === 2 ? 2 : 1,
              channel_name: null,
              route_attempts: [
                {
                  attempt: 1,
                  channel_id: 1,
                  route_id: 'preview-route-1',
                  status_code: index === 2 ? 429 : null,
                  error_kind: index === 2 ? 'rate_limited' : null,
                  delivery: index === 2 ? 'not_sent' : null,
                },
                ...(index === 2
                  ? [
                      {
                        attempt: 2,
                        channel_id: 2,
                        route_id: 'preview-route-2',
                        status_code: 429,
                        error_kind: 'rate_limited',
                        delivery: 'not_sent',
                      },
                    ]
                  : []),
              ],
            }
          : {}),
        other: JSON.stringify(other),
      }
    }
  )
}

export function withUsageLogsReviewFixtures(
  fallback: AxiosAdapter
): AxiosAdapter {
  return async (config) => {
    const url = new URL(config.url ?? '', window.location.origin)
    if (
      (config.method ?? 'get').toUpperCase() !== 'GET' ||
      url.origin !== window.location.origin
    ) {
      return fallback(config)
    }
    const native =
      /^\/core\/v1\/usage\/(admin|self)\/(logs|stats|request|export\.csv)$/.exec(
        url.pathname
      )
    const self =
      url.pathname === '/api/log/self' || url.pathname === '/api/log/self/stat'
    if (!native && !self) return fallback(config)
    const administrator = native?.[1] === 'admin'
    const action =
      native?.[2] ?? (url.pathname.endsWith('/stat') ? 'stats' : 'logs')
    const query = url.searchParams
    const observedAt = Math.floor(Date.now() / 1000)
    const end = Number(query.get('end_timestamp') ?? observedAt)
    const start = Number(
      query.get('start_timestamp') ?? Math.max(0, end - 31 * 86400)
    )
    const allRows = records(administrator)
    type ReviewRow = (typeof allRows)[number]
    const isPending = (row: ReviewRow) =>
      ['reserved', 'streaming', 'reconcile'].includes(row.state)
    const isSuccessful = (row: ReviewRow) =>
      row.state === 'settled' && row.outcome === 'completed'
    const isRefunded = (row: ReviewRow) =>
      row.state === 'settled' && (row.refunded_credits ?? 0) > 0
    const status = query.get('status')
    let rows = allRows.filter((row) => {
      const matches = [
        'model_name',
        'token_name',
        'group',
        'request_id',
        'upstream_request_id',
        'username',
      ].every((key) => {
        const value = query.get(key)
        return !value || String(row[key as keyof typeof row] ?? '') === value
      })
      const statuses = {
        success: isSuccessful(row),
        error: !isPending(row) && !isSuccessful(row),
        pending: isPending(row),
        refunded: isRefunded(row),
      }
      return (
        matches &&
        (action === 'request' ||
          (row.created_at >= start && row.created_at <= end)) &&
        (!status || statuses[status as keyof typeof statuses]) &&
        (!query.get('channel') ||
          query.get('channel') === '0' ||
          row.channel === Number(query.get('channel'))) &&
        (!query.get('type') ||
          query.get('type') === '0' ||
          row.type === Number(query.get('type'))) &&
        (!query.get('min_duration_ms') ||
          (typeof row.duration_ms === 'number' &&
            row.duration_ms >= Number(query.get('min_duration_ms')))) &&
        (!query.get('max_duration_ms') ||
          (typeof row.duration_ms === 'number' &&
            row.duration_ms <= Number(query.get('max_duration_ms'))))
      )
    })
    if (query.get('sort_order') === 'asc') rows = rows.reverse()
    let data: unknown
    if (action === 'stats') {
      const sum = (key: keyof ReviewRow, matchingRows = rows) =>
        matchingRows.reduce(
          (total, row) =>
            total + (typeof row[key] === 'number' ? (row[key] as number) : 0),
          0
        )
      const sumOther = (
        matchingRows: ReviewRow[],
        key: string,
        requireComplete = false
      ) => {
        let total = 0
        for (const row of matchingRows) {
          const value: unknown = JSON.parse(row.other)[key]
          if (typeof value === 'number') total += value
          else if (requireComplete) return null
        }
        return total
      }
      const settled = rows.filter((row) => row.state === 'settled')
      const measuredText = settled.filter((row) => row.token_usage_available)
      const pending = rows.filter(isPending)
      const recent = settled.filter(
        (row) =>
          row.created_at > observedAt - 60 && row.created_at <= observedAt
      )
      const inputTokens = sum('prompt_tokens', measuredText)
      const outputTokens = sum('completion_tokens', measuredText)
      const totalTokens = inputTokens + outputTokens
      const grossCredits = sum('gross_settled_credits', settled)
      const refundedCredits = sum('refunded_credits', settled)
      const netCredits = grossCredits - refundedCredits
      const durations = rows
        .flatMap((row) =>
          typeof row.duration_ms === 'number' ? [row.duration_ms] : []
        )
        .sort((a, b) => a - b)
      const rateEnd = Math.min(end, observedAt)
      const seconds = Math.max(1, rateEnd - start + 1)
      data = {
        quota: netCredits,
        quota_usd: (netCredits / 500000).toFixed(6),
        quota_money_available: true,
        quota_money_basis: 'captured_credits_per_usd',
        wallet_debit_supported: false,
        rpm: recent.length,
        tpm: sum('prompt_tokens', recent) + sum('completion_tokens', recent),
        billing_basis: 'net_consumption',
        gross_settled_credits: grossCredits,
        refunded_credits: refundedCredits,
        net_consumption_credits: netCredits,
        total_count: rows.length,
        success_count: rows.filter(isSuccessful).length,
        error_count:
          rows.length - rows.filter(isSuccessful).length - pending.length,
        pending_count: pending.length,
        refunded_count: rows.filter(isRefunded).length,
        input_tokens: inputTokens,
        output_tokens: outputTokens,
        total_tokens: totalTokens,
        average_duration_ms: durations.length
          ? durations.reduce<number>((total, duration) => total + duration, 0) /
            durations.length
          : null,
        p95_duration_ms: durations.length
          ? durations[Math.ceil(durations.length * 0.95) - 1]
          : null,
        duration_sample_count: durations.length,
        duration_percentile_scope: 'all_samples',
        cache_read_tokens: sumOther(measuredText, 'cache_tokens'),
        cache_write_tokens: sumOther(
          measuredText,
          'cache_creation_tokens',
          true
        ),
        cache_write_5m_tokens: sumOther(
          measuredText,
          'cache_creation_tokens_5m',
          true
        ),
        cache_write_1h_tokens: sumOther(
          measuredText,
          'cache_creation_tokens_1h',
          true
        ),
        cache_write_12h_tokens: sumOther(
          measuredText,
          'cache_creation_tokens_12h',
          true
        ),
        reserved_credits: sumOther(pending, 'reserved_credits'),
        incomplete_count: pending.length,
        requests_per_minute: (rows.length * 60) / seconds,
        tokens_per_minute: (totalTokens * 60) / seconds,
        rate_basis: 'selected_window',
        rate_window_seconds: 60,
        window_start: start,
        window_end: rateEnd,
        token_count_scope: 'settled_text_requests',
        settlement_scope: 'settled_requests',
        spending_scope: 'all_payers',
        usage_scope: administrator
          ? 'administrator_all_accounts'
          : 'actor_personal_key',
      }
    } else if (action === 'request') {
      data = rows[0]
    } else if (action === 'export.csv') {
      return {
        config,
        status: 200,
        statusText: 'OK',
        headers: { 'content-type': 'text/csv' },
        data: new Blob(
          [
            'request_id,model,net_consumption_credits\r\n',
            ...rows.map(
              (row) =>
                `${row.request_id},${row.model_name},${row.quota ?? ''}\r\n`
            ),
          ],
          { type: 'text/csv' }
        ),
      }
    } else {
      const page = Number(query.get('p') ?? 1)
      const size = Number(query.get('page_size') ?? 20)
      data = {
        total: rows.length,
        page,
        page_size: size,
        items: rows.slice((page - 1) * size, page * size),
      }
    }
    return {
      config,
      status: 200,
      statusText: 'OK',
      headers: {},
      data: { success: true, data },
    }
  }
}
