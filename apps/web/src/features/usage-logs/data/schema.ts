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
/**
 * Zod schemas for common logs
 * This file should only contain Zod schemas and types inferred from them
 */
import { z } from 'zod'

// Usage log schema
export const usageLogSchema = z.object({
  id: z.number(),
  source_log_id: z.number().int().safe().optional(),
  user_id: z.number(),
  created_at: z.number(),
  type: z.number(),
  content: z.string(),
  username: z
    .string()
    .nullish()
    .transform((value) => value ?? ''),
  token_name: z.string().default(''),
  model_name: z.string().default(''),
  quota: z.number().nullable().default(null),
  quota_usd: z.string().optional(),
  quota_money_available: z.boolean().optional(),
  quota_money_basis: z.string().optional(),
  billing_basis: z.string().optional(),
  state: z.string().optional(),
  outcome: z.string().nullish(),
  request_result: z.enum(['success', 'error', 'disconnected']).nullish(),
  settlement_status: z
    .enum(['pending', 'settled', 'released', 'refunded'])
    .optional(),
  gross_settled_credits: z.number().int().safe().nonnegative().nullish(),
  refunded_credits: z.number().int().safe().nonnegative().nullish(),
  net_consumption_credits: z.number().int().safe().nonnegative().nullish(),
  pricing_dimension: z.enum(['image', 'text', 'tool']).optional(),
  billing_method: z.string().optional(),
  price_version: z.string().nullish(),
  tool_kind: z.enum(['deepseek_search', 'deepseek_fetch']).nullish(),
  tool_requests: z.number().int().min(0).max(1).nullish(),
  completed_requests: z.number().int().min(0).max(1).nullish(),
  grounded_requests: z.number().int().min(0).max(1).nullish(),
  token_usage_available: z.boolean().optional(),
  output_image_count: z.number().int().safe().positive().nullish(),
  settlement_timeline: z
    .object({
      events: z.array(
        z.object({
          kind: z.enum(['reserve_total', 'captured', 'released', 'refunded']),
          at: z.number().int().safe().nonnegative(),
          credits: z.number().int().safe().nonnegative(),
        })
      ),
      truncated: z.boolean(),
      wait_seconds_at_read: z.number().int().safe().nonnegative().nullable(),
      hold_retained_at: z.number().int().safe().nonnegative().nullable(),
    })
    .optional(),
  platform_sponsored: z.boolean().optional(),
  payer_kind: z.enum(['personal', 'team']).optional(),
  wallet_debit_supported: z.boolean().optional(),
  prompt_tokens: z.number().nullable().default(null),
  completion_tokens: z.number().nullable().default(null),
  use_time: z.number().nullable().default(null),
  duration_ms: z.number().finite().nonnegative().nullish(),
  first_token_ms: z.number().finite().nonnegative().nullish(),
  diagnostics_available: z.boolean().optional(),
  status_code: z.number().int().min(100).max(599).nullish(),
  provider_http_status: z.number().int().min(100).max(599).nullish(),
  relay_diagnostics: z
    .object({
      inbound_protocol: z.enum(['chat', 'responses', 'anthropic', 'gemini']),
      error_code: z.string().nullish(),
      error_stage: z.string().nullish(),
      sse_frame: z
        .object({
          channel_id: z.number().int().safe().positive().nullable(),
          upstream_protocol: z.enum([
            'chat',
            'responses',
            'anthropic',
            'gemini',
          ]),
          ordinal: z.number().int().safe().positive(),
          event: z.string(),
          converter: z.string().nullable(),
        })
        .nullish(),
    })
    .nullish(),
  route_attempts: z
    .array(
      z.object({
        attempt: z.number().int().nonnegative(),
        channel_id: z.number().int().safe().nonnegative(),
        route_id: z.string(),
        upstream_model: z.string().optional(),
        protocol: z.string().optional(),
        inbound_protocol: z
          .enum(['chat', 'responses', 'anthropic', 'gemini'])
          .nullish(),
        protocol_path: z.enum(['same_protocol', 'converted']).nullish(),
        status_code: z.number().int().nullable(),
        error_kind: z.string().nullable(),
        delivery: z.string().nullable(),
      })
    )
    .optional(),
  is_stream: z.boolean().nullable().default(false),
  channel: z.number().nullable().default(null),
  channel_name: z.string().nullish().default(''),
  token_id: z.number().nullish().default(null),
  group: z.string().default(''),
  ip: z.string().default(''),
  other: z.string().default(''),
  request_id: z.string().default(''),
  upstream_request_id: z.string().nullable().default(''),
})

export type UsageLog = z.infer<typeof usageLogSchema>
