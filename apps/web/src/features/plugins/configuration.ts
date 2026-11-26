/* Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later */

// Host field names are setup instructions, with no live credential values.
const hostFields: Record<string, readonly string[]> = {
  acquisition: ['host.public_origin', 'acquisition.database_url_file'],
  assistant: ['core.assistant_token_file', 'assistant.database_url_file'],
  checkins: ['core.checkin_token_file', 'checkins.database_url_file'],
  'hero-sms': ['hero_sms.core_socket', 'hero_sms.core_token_file'],
  operations: ['operations.database_url_file'],
}

export function pluginHostFields(name: string) {
  return Object.hasOwn(hostFields, name) ? hostFields[name] : undefined
}
