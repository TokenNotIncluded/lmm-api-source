/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */

// These are the existing administrative routes and the host's public TOML
// field names. They describe setup requirements, never live secret values.
const configuration: Record<
  string,
  { href?: string; fields: readonly string[] }
> = {
  acquisition: {
    href: '/system-settings/billing/quota',
    fields: ['host.public_origin', 'acquisition.database_url_file'],
  },
  assistant: {
    href: '/system-settings/content/assistant',
    fields: ['core.assistant_token_file', 'assistant.database_url_file'],
  },
  checkins: {
    href: '/system-settings/billing/checkin',
    fields: ['core.checkin_token_file', 'checkins.database_url_file'],
  },
  'hero-sms': {
    href: '/system-settings/operations/hero-sms',
    fields: ['hero_sms.core_socket', 'hero_sms.core_token_file'],
  },
  operations: {
    href: '/system-settings/operations/scripts',
    fields: ['operations.database_url_file'],
  },
  payments: {
    href: '/system-settings/billing/payment',
    fields: [
      'core.payment_token_file',
      'payments.database_url_file',
      'payments.config_file',
    ],
  },
}

export function pluginConfiguration(name: string) {
  return configuration[name]
}
