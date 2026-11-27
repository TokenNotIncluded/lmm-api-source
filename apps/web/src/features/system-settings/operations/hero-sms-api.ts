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
import * as z from 'zod'

import { api } from '@/lib/api'

const heroSmsStatus = z.object({
  api_key_configured: z.boolean(),
  credential_managed_by_file: z.literal(true),
  pending_work: z.boolean(),
  purchases_available: z.boolean(),
})

interface HeroSmsEnvelope<T> {
  success: boolean
  message?: string
  data: T
}

async function unwrap<T>(request: Promise<{ data: HeroSmsEnvelope<T> }>) {
  const { data } = await request
  if (!data.success)
    throw new Error(data.message || 'HeroSMS settings request failed')
  return data.data
}

export async function getHeroSmsSettings() {
  return heroSmsStatus.parse(
    await unwrap<unknown>(
      api.get('/api/option/hero-sms', {
        skipBusinessError: true,
        skipErrorHandler: true,
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
