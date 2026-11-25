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
import * as z from 'zod'

import { api } from '@/lib/api'

const heroSmsSettingsResponseSchema = z.object({
  revision: z.string().regex(/^(0|[1-9][0-9]*)$/),
  enabled: z.boolean(),
  email_enabled: z.boolean(),
  sms_enabled: z.boolean(),
  api_key_configured: z.boolean(),
  credential_managed_by_file: z.literal(true),
  policy_managed_by_file: z.boolean().optional(),
  pending_work: z.boolean(),
  purchases_available: z.boolean(),
  currency: z.literal('USD'),
  currency_code: z.literal(840),
  price_multiplier: z.string().regex(/^\d+(?:\.\d{1,6})?$/),
})

export type HeroSmsSettingsResponse = z.infer<
  typeof heroSmsSettingsResponseSchema
>

export interface HeroSmsSettingsFormValues {
  revision: string
  enabled: boolean
  emailEnabled: boolean
  smsEnabled: boolean
  priceMultiplier: number
}

export interface HeroSmsSettingsUpdateRequest {
  expected_revision: string
  enabled: boolean
  email_enabled: boolean
  sms_enabled: boolean
  price_multiplier: string
}

interface HeroSmsEnvelope<T> {
  success: boolean
  code?: string
  message?: string
  data: T
}

async function unwrap<T>(request: Promise<{ data: HeroSmsEnvelope<T> }>) {
  const response = await request
  if (!response.data.success) {
    const error = new Error(
      response.data.message || 'HeroSMS settings request failed'
    )
    Object.assign(error, { code: response.data.code })
    throw error
  }
  return response.data.data
}

export function toHeroSmsSettingsFormValues(
  data: HeroSmsSettingsResponse
): HeroSmsSettingsFormValues {
  return {
    revision: data.revision,
    enabled: Boolean(data.enabled),
    emailEnabled: data.email_enabled !== false,
    smsEnabled: data.sms_enabled === true,
    priceMultiplier: Number(data.price_multiplier || 1),
  }
}

export function serializeHeroSmsSettingsUpdate(
  values: HeroSmsSettingsFormValues
): HeroSmsSettingsUpdateRequest {
  return {
    expected_revision: values.revision,
    enabled: values.enabled,
    email_enabled: values.emailEnabled,
    sms_enabled: values.smsEnabled,
    price_multiplier: String(values.priceMultiplier),
  }
}

export function getHeroSmsPreviewCustomerPrice(multiplier: number) {
  return Number((multiplier * 1).toFixed(2))
}

export function parseHeroSmsSettingsError(error: unknown) {
  if (isAxiosError(error)) {
    return {
      status: error.response?.status,
      code: (error.response?.data as { code?: string } | undefined)?.code,
      message:
        (error.response?.data as { message?: string } | undefined)?.message ||
        error.message ||
        'HeroSMS settings request failed',
    }
  }

  if (error instanceof Error) {
    return {
      message: error.message,
      code:
        'code' in error && typeof error.code === 'string'
          ? error.code
          : undefined,
    }
  }

  return { message: 'HeroSMS settings request failed' }
}

export async function getHeroSmsSettings() {
  return heroSmsSettingsResponseSchema.parse(
    await unwrap<unknown>(
      api.get('/api/option/hero-sms', {
        skipBusinessError: true,
        skipErrorHandler: true,
      })
    )
  )
}

export async function updateHeroSmsSettings(values: HeroSmsSettingsFormValues) {
  return heroSmsSettingsResponseSchema.parse(
    await unwrap<unknown>(
      api.put('/api/option/hero-sms', serializeHeroSmsSettingsUpdate(values), {
        skipBusinessError: true,
        skipErrorHandler: true,
        skipAuthRefresh: true,
      })
    )
  )
}

export async function testHeroSmsConnection() {
  const result = await unwrap<{ ok: boolean }>(
    api.post(
      '/api/option/hero-sms/test',
      {},
      {
        skipBusinessError: true,
        skipErrorHandler: true,
        skipAuthRefresh: true,
      }
    )
  )
  if (result.ok !== true) throw new Error('HeroSMS connection test failed')
  return result
}
