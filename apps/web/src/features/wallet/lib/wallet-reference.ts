/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import { formatExactQuotaInCurrency } from '@/lib/currency'
import type { CurrencyConfig } from '@/stores/system-config-store'
import type { WalletDisplayCurrency } from '@/stores/wallet-currency-preference-store'

/** Current-rate reference only; this does not establish a historical cash basis. */
export function walletReferenceValue(
  quota: number | null,
  currency: WalletDisplayCurrency,
  config: CurrencyConfig,
  locale?: string
): string | null {
  if (
    currency === 'CREDIT' ||
    quota === null ||
    !Number.isSafeInteger(quota) ||
    config.creditUnitSchemaVersion !== 2
  ) {
    return null
  }
  const value = formatExactQuotaInCurrency(
    String(quota),
    currency,
    { locale },
    config
  )
  return value === '-' ? null : value
}
