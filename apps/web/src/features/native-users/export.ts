/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */
import type { NativeUser } from './contract'

function cell(value: string): string {
  // CSV quoting does not prevent spreadsheet formula execution.
  const safe = /^[\s\uFEFF]*[=+@-]/u.test(value) ? `'${value}` : value
  return `"${safe.replaceAll('"', '""')}"`
}

// A text marker prevents spreadsheet applications from rounding long IDs and
// credit integers. No amount is ever converted to a JavaScript Number.
const exact = (value: string) => `'${value}`
const time = (value: number | null) =>
  value === null ? '' : exact(String(value))

export function nativeUsersCsv(
  users: readonly NativeUser[],
  headers: readonly string[]
): string {
  const records = users.map((user) => {
    const wallet = user.wallet
    return [
      exact(user.id),
      String(user.profile_available),
      user.username ?? '',
      user.display_name ?? '',
      user.email ?? '',
      user.platform_role,
      user.active ? 'active' : 'disabled',
      user.account_active ? 'active' : 'inactive',
      String(user.service_level),
      String(user.developer_access_granted),
      JSON.stringify(user.groups),
      time(user.registered_at_unix_ms),
      time(user.core_account_created_at_unix_ms),
      wallet.status,
      wallet.status === 'available' ? wallet.unit : '',
      ...(wallet.status === 'available'
        ? [
            wallet.available_units,
            wallet.wallet_units,
            wallet.reserved_units,
            wallet.debt_units,
          ].map(exact)
        : ['', '', '', '']),
    ]
  })
  return `\uFEFF${[headers, ...records].map((record) => record.map(cell).join(',')).join('\r\n')}\r\n`
}
