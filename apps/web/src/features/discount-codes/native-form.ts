/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { nativeDiscountCode } from '@/features/wallet/lib/native-checkout'

import { discountDefinitionSchema, type NativeDiscount } from './native-api'

export type DiscountForm = {
  code: string
  name: string
  percent: string
  minimum: string
  maximum: string
  starts: string
  ends: string
  enabled: boolean
  count: string
  source:
    | { kind: 'new' }
    | {
        kind: 'existing'
        starts: string
        ends: string
        localStarts: string
        localEnds: string
      }
}
export const emptyDiscountForm: DiscountForm = {
  code: '',
  name: '',
  percent: '10',
  minimum: '0',
  maximum: '0',
  starts: '',
  ends: '',
  enabled: true,
  count: '1',
  source: { kind: 'new' },
}
const termsSchema = discountDefinitionSchema.omit({ code: true })
const MAX_FOUR_DIGIT_YEAR_SECONDS = 253402300799n
function epoch(value: string) {
  if (!value) return '0'
  const milliseconds = new Date(value).getTime()
  if (!Number.isSafeInteger(milliseconds) || milliseconds < 0) {
    throw new Error('Invalid date')
  }
  return BigInt(Math.floor(milliseconds / 1000)).toString()
}
function localTime(value: string) {
  const seconds = BigInt(value)
  if (seconds === 0n) return ''
  if (seconds > MAX_FOUR_DIGIT_YEAR_SECONDS) {
    throw new Error('Date outside supported input range')
  }
  const date = new Date(Number(seconds) * 1000)
  const pad = (n: number) => n.toString().padStart(2, '0')
  return `${date.getFullYear().toString().padStart(4, '0')}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`
}
export function editDiscountForm(code: NativeDiscount): DiscountForm {
  const starts = localTime(code.starts_at)
  const ends = localTime(code.ends_at)
  return {
    code: code.code,
    name: code.name,
    percent: String(code.discount_percent),
    minimum: code.min_credit_units,
    maximum: code.max_uses,
    starts,
    ends,
    enabled: code.enabled,
    count: '1',
    source: {
      kind: 'existing',
      starts: code.starts_at,
      ends: code.ends_at,
      localStarts: starts,
      localEnds: ends,
    },
  }
}
export function discountTerms(form: DiscountForm) {
  const terms = termsSchema.parse({
    name: form.name,
    discount_percent: Number(form.percent),
    min_credit_units: form.minimum.trim(),
    max_uses: form.maximum.trim(),
    starts_at:
      form.source.kind === 'existing' && form.starts === form.source.localStarts
        ? form.source.starts
        : epoch(form.starts),
    ends_at:
      form.source.kind === 'existing' && form.ends === form.source.localEnds
        ? form.source.ends
        : epoch(form.ends),
    enabled: form.enabled,
  })
  if (
    terms.ends_at !== '0' &&
    BigInt(terms.ends_at) <= BigInt(terms.starts_at)
  ) {
    throw new Error('Invalid date window')
  }
  return terms
}
export function discountDefinition(form: DiscountForm) {
  return discountDefinitionSchema.parse({
    ...discountTerms(form),
    code: nativeDiscountCode(form.code) ?? '',
  })
}
