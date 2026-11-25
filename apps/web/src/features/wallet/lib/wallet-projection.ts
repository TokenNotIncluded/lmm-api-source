/*
Copyright (C) 2026 LIghtJUNction

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.
*/
import type { AuthUser } from '@/stores/auth-store'

export type WalletProjectionUser = Pick<
  AuthUser,
  | 'quota'
  | 'financial_projection_available'
  | 'usage_projection_available'
  | 'normalized_used_quota'
  | 'used_quota'
  | 'request_count'
  | 'wallet_unit'
  | 'wallet_spendable_quota'
  | 'wallet_reserved_quota'
  | 'wallet_debt'
  | 'wallet_money_basis'
  | 'wallet_money_available'
>

export function exactCredit(value: unknown): number | null {
  return typeof value === 'number' && Number.isSafeInteger(value) ? value : null
}

export function walletProjection(
  user: WalletProjectionUser | null | undefined
) {
  const financial =
    user?.wallet_unit === 'core_credit' &&
    user.financial_projection_available === true
  const available = financial ? exactCredit(user.quota) : null
  const bucket = (value: unknown) => {
    const exact = exactCredit(value)
    return financial && exact !== null && exact >= 0 ? exact : null
  }
  // Core omits raw counters when the personal historical opening is unknown.
  // The usage flag only describes the separate normalized credit amount.
  const history =
    typeof user?.usage_projection_available === 'boolean' &&
    (bucket(user.used_quota) !== null ||
      (user.usage_projection_available &&
        bucket(user.normalized_used_quota) !== null))
  const requests =
    typeof user?.usage_projection_available === 'boolean'
      ? bucket(user.request_count)
      : null
  return {
    native: true as const,
    available,
    spendable: bucket(user?.wallet_spendable_quota),
    reserved: bucket(user?.wallet_reserved_quota),
    debt: bucket(user?.wallet_debt),
    history,
    requests,
    // A read-only wallet DTO is not a checkout capability or a money quote.
    readOnly: true as const,
  }
}

export function nativeCreditDisplay(
  value: number | null,
  creditLabel: string,
  unknownLabel: string
) {
  return value === null
    ? unknownLabel
    : `${value.toLocaleString()} ${creditLabel}`
}
