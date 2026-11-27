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

const legacyPanels = new Map([
  ['/system-settings/content/assistant', 'assistant'],
  ['/system-settings/billing/checkin', 'checkins'],
  ['/system-settings/operations/hero-sms', 'hero-sms'],
  ['/system-settings/operations/scripts', 'operations'],
  ['/system-settings/site/rss', 'rss'],
  ['/system-settings/site/ai-directory-ads', 'directory-ads'],
  ['/system-settings/site/ai-directory', 'community'],
  ['/system-settings/site/header-navigation', 'header-navigation'],
  ['/system-settings/models/model-deployment', 'model-deployment'],
  ['/system-settings/security/market-ai-review', 'toolmarket'],
])

export function legacyPluginPanel(path: string, hash = '') {
  const pathname = path.replace(/\/+$/, '')
  if (
    pathname === '/system-settings/billing/payment' &&
    hash.replace(/^#/, '') === 'merchant-store'
  ) {
    return 'store'
  }
  return legacyPanels.get(pathname)
}

export function configurationOwner(panel: string | undefined) {
  if (panel === 'rss' || panel === 'model-deployment') return 'operations'
  if (panel === 'directory-ads' || panel === 'header-navigation') {
    return 'community'
  }
  if (panel === 'store-review') return 'store'
  if (panel === 'community' || panel === 'store' || panel === 'toolmarket') {
    return panel
  }
  return panel && pluginHostFields(panel) ? panel : undefined
}

export function canOpenConfiguration(
  panel: unknown,
  root: boolean
): panel is string {
  if (typeof panel !== 'string' || !configurationOwner(panel)) return false
  const owner = configurationOwner(panel)
  return (
    root ||
    (owner !== 'community' &&
      panel !== 'checkins' &&
      panel !== 'hero-sms' &&
      panel !== 'store' &&
      panel !== 'rss' &&
      panel !== 'model-deployment')
  )
}
